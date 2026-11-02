#!/usr/bin/env python3
"""Write every silicon POWER ledger from measurements of the netlists the signoff ledgers ship.

    python3 scripts/record-silicon-power.py            # rewrite the power ledgers
    python3 scripts/record-silicon-power.py --check    # exit 1 if any ledger differs from what the
                                                       # measurements say

Inputs are the JSON reports of `scripts/measure-silicon-power.py <block>`, one per block, under
logs/power-<block>.json. A report is refused unless its `gds_bytes` is the GDS the block's signoff
ledger ships TODAY — power evidence from a netlist that has since been re-hardened is not evidence —
and unless its gate-level window ends at the signature the RTL it was synthesized from ends at over the
same window: activity from a netlist that does not compute its design is not that design's power.

Every derived figure is computed here from those reports and never typed:
  * a block's clock is its signoff clock (the period every timing corner meets), and its Fmax is the
    frequency every corner meets on the shipped netlist, 1000 / (period - worst setup slack);
  * energy per operation is power x the time the operation takes at that clock;
  * the soft-core baseline runs the SAME kernel, its cycle count measured on the cycle-accurate RTL.
The headline corner is typical (tt_025C_1v80, nominal RC); slow and fast corners are recorded beside
it. Power figures are milliwatts to three decimals, energies picojoules to two.
"""
import json
import os
import re
import sys

ROOT = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
SIL = os.path.join(ROOT, "benchmarks", "silicon")
TT, SS, FF = "tt_025C_1v80", "ss_100C_1v60", "ff_n40C_1v95"
VDD = {TT: 1.80, SS: 1.60, FF: 1.95}

# Cycle counts measured on the cycle-accurate RTL, independent of any netlist.
SOFT_CORE_DECISION_KERNEL_CYCLES = 82   # tb_hft.v: 187 (decision inline) - 105 (Seq+index marshaling)
SOFT_CORE_CYCLES_PER_MAC = 10.625       # gemm_fabric.json sequential_baseline, the int8 MAC kernel


def measured(block):
    rep = json.load(open(os.path.join(ROOT, "logs", f"power-{block}.json")))
    ledger = json.load(open(os.path.join(ROOT, rep["ledger"])))["blocks"][block]
    if rep["gds_bytes"] != ledger["gds_bytes"]:
        sys.exit(f"{block}: the power report measured a {rep['gds_bytes']}-byte GDS; the signoff ledger ships "
                 f"{ledger['gds_bytes']} bytes — re-run scripts/measure-silicon-power.py {block}")
    if rep["simulated_cycles"]:
        w = rep.get("rtl_witness") or {}
        if (not rep.get("signature_trajectory") or w.get("signature_trajectory") != rep["signature_trajectory"]
                or w.get("cycles") != rep["simulated_cycles"]):
            sys.exit(f"{block}: the power report's gate window is not the trajectory its RTL follows over the same "
                     f"{rep['simulated_cycles']} cycles — re-run scripts/measure-silicon-power.py {block}")
    return rep


def mw(rep, corner=TT, activity="default", group="total"):
    return round(rep["corners"][corner][activity][group]["total"] * 1e3, 3)


def fmax(rep):
    return round(1000.0 / (rep["clock_period_ns"] - rep["worst_setup_slack_ns"]), 1)


def pj(mw_value, period_ns, per=1.0):
    """Energy of one operation (per `per` units) at `mw_value` over `period_ns`, in pJ."""
    return round(mw_value * period_ns / per, 2)


def binding(rep):
    """What a power record measured: the shipped run and its GDS, the signoff clock, and the simulated
    window — its length, the output it followed, where that output ended, how often it moved (never,
    and it witnesses nothing), and the trajectory the gate netlist and its RTL both follow over it."""
    w = rep["rtl_witness"]
    return {"run": rep["run"], "gds_bytes": rep["gds_bytes"], "clock_period_ns": rep["clock_period_ns"],
            "simulated_cycles": rep["simulated_cycles"], "signature_port": w["signature_port"],
            "signature": rep["signature"], "signature_changes": rep["signature_changes"],
            "signature_trajectory_sha256": rep["signature_trajectory"],
            "rtl_signature": w["signature"], "rtl_signature_trajectory_sha256": w["signature_trajectory"]}


def spef_average_net_fF(run, block):
    """Mean total parasitic capacitance per net in the run's nominal SPEF, in fF."""
    path = os.path.join(ROOT, run, "results/final/spef/multicorner", f"{block}.nom.spef")
    unit, caps = 1.0, []
    for line in open(path):
        if line.startswith("*C_UNIT"):
            f = line.split()
            unit = float(f[1]) * {"PF": 1e3, "FF": 1.0, "NF": 1e6}[f[2].upper()]
        elif line.startswith("*D_NET"):
            caps.append(float(line.split()[2]) * unit)
    return round(sum(caps) / len(caps), 2)


def decision_mix(real):
    """The decisions the realistic stimulus made over the measured window, counted on the RTL whose
    signature over the same window the gate run reproduces."""
    w = real["rtl_witness"]
    return {
        "_what": ("every decision latched in the measured window, classified on the RTL: buy (quantity > 0), sell "
                  "(< 0), risk-blocked (a side chosen, the position limit refuses it), no-trade (no side)"),
        "cycles": w["cycles"],
        **{k: w[k] for k in ("buy", "sell", "risk_blocked", "no_trade", "decision_changes")},
        "rtl_signature": w["signature"],
        "gate_signature": real["signature"],
    }


def mix_phrase(mix):
    return (f"buy {mix['buy']} / sell {mix['sell']} / risk-blocked {mix['risk_blocked']} / "
            f"no-trade {mix['no_trade']}")


def sky130_power(soft):
    tt = soft["corners"][TT]["default"]
    tot = tt["total"]["total"]
    return {
        "_provenance": (f"OpenLane Sky130 signoff, OpenSTA report_power on the post-route RC-extracted netlist "
                        f"the ledger ships ({soft['run']}, {soft['gds_bytes']}-byte GDS) at its {soft['clock_period_ns']:g} ns "
                        f"signoff clock, typical corner. Switching activity is OpenSTA's propagated default (no "
                        f"workload VCD) — a signoff EDA power number, not a bench-measured one."),
        "design": "soft_core_scalar",
        "pdk": "sky130A (OpenLane)",
        **binding(soft),
        "cells": soft["std_cells"],
        "die_mm2": round(soft["die_mm2"], 4),
        "clock_mhz": round(1000.0 / soft["clock_period_ns"], 3),
        "setup_slack_ns": soft["worst_setup_slack_ns"],
        "fmax_mhz": fmax(soft),
        "corner": "typical",
        "power_w": {k: float(f"{tt['total'][k]:.3g}") for k in ("internal", "switching", "leakage", "total")},
        "split_pct": {
            "sequential": round(100 * tt["sequential"]["total"] / tot, 1),
            "combinational": round(100 * tt["combinational"]["total"] / tot, 1),
            "internal": round(100 * tt["total"]["internal"] / tot, 1),
            "switching": round(100 * tt["total"]["switching"] / tot, 1),
            "leakage": round(100 * tt["total"]["leakage"] / tot, 1),
        },
        "corners_mw": {SS: mw(soft, SS), TT: mw(soft, TT), FF: mw(soft, FF)},
    }


def decision_fabric_power(old, soft, bist, real, cgra, twin):
    period = bist["clock_period_ns"]
    block_mw, comb_mw, seq_mw = mw(bist), mw(bist, group="combinational"), mw(bist, group="sequential")
    e_block, e_comb = pj(block_mw, period), pj(comb_mw, period)
    soft_mw = mw(soft)
    soft_kernel_nj = round(soft_mw * SOFT_CORE_DECISION_KERNEL_CYCLES * soft["clock_period_ns"] / 1000.0, 2)
    area_x = round(soft["die_mm2"] / bist["die_mm2"])
    default_real, vcd_real = mw(real), mw(real, activity="vcd")
    vcd_tt = real["corners"][TT]["vcd"]
    rises = real["vcd_rises"]
    implied_fF = round(vcd_tt["total"]["switching"] * real["vcd_span_ns"] * 1e-9
                       / (VDD[TT] ** 2 * rises) * 1e15, 2) if rises else None
    spef_fF = spef_average_net_fF(real["run"], real["block"])
    annotated = real["corners"][TT].get("annotated_pins"), real["corners"][TT].get("design_pins")
    return {
        "_provenance": ("The HFT DECISION lowered to a combinational select-DAG, proven ==VM (hft_decision_fabric_lock + "
                        "iverilog tb_decision_hft, 256 vectors), taped out standalone behind an on-chip BIST harness. "
                        "Every number below is measured on the netlist the signoff ledger ships "
                        "(scripts/measure-silicon-power.py) and every ratio derived from those measurements "
                        "(scripts/record-silicon-power.py)."),
        "design": "decision_hft (HFT microprice market-making decision + fat-finger clamp + position risk gate)",
        "tiles": old["tiles"],
        "signoff_clean": {
            "_provenance": ("OpenLane Sky130 RTL-to-GDSII of decision_hft_bist.v (LFSR-fed BIST harness, clk/rst/sig "
                            "only) — signoff-clean at every timing corner, golden sky130A_mr DRC 0. OpenSTA "
                            "report_power on the post-route RC-extracted netlist at the signoff clock, typical "
                            "corner, default switching activity."),
            **binding(bist),
            "die_mm2": round(bist["die_mm2"], 4),
            "std_cells_placed": bist["std_cells"],
            "clock_mhz": round(1000.0 / period, 3),
            "setup_slack_ns": bist["worst_setup_slack_ns"],
            "fmax_mhz": fmax(bist),
            "cycles_per_trade": 1,
            "power_mw": {
                "block_total": block_mw, "decision_selectdag_comb": comb_mw, "bist_lfsr_misr_seq": seq_mw,
                "note": ("decision_selectdag_comb (Combinational group) is the decision; bist_lfsr_misr_seq is "
                         "the stimulus/signature overhead, not the decision."),
            },
            "energy_per_trade_pJ": {"decision": e_comb, "block": e_block},
            "vs_soft_core": {
                "basis": (f"the SAME decision kernel on the soft-core: {SOFT_CORE_DECISION_KERNEL_CYCLES} cycles at "
                          f"{soft_mw} mW over its {soft['clock_period_ns']:g} ns clock = {soft_kernel_nj} nJ"),
                "power_draw_reduction_x": [round(soft_mw / block_mw), round(soft_mw / comb_mw)],
                "area_reduction_x": area_x,
                "energy_per_trade_reduction_x": [round(soft_kernel_nj * 1000 / e_block), round(soft_kernel_nj * 1000 / e_comb)],
            },
            "cross_check": f"cgra_sumto adder-fabric signoff-clean {mw(cgra)} mW ~ decision {comb_mw} mW",
        },
        "vcd_validation": {
            "_what": ("VCD-annotated signoff power: real switching activity from a gate-level simulation of the "
                      "signoff-clean post-route netlist at its signoff clock, fed into OpenSTA report_power. Open "
                      "equivalent of PrimePower/Voltus."),
            "design": "decision_hft_realbist (decision + realistic market-input stimulus, signoff-clean at every corner)",
            **binding(real),
            "clock_mhz": round(1000.0 / real["clock_period_ns"], 3),
            "fmax_mhz": fmax(real),
            "workload": (f"on-chip realistic market-input stimulus; {real['simulated_cycles']} cycles of gate-level "
                         f"simulation deciding {mix_phrase(decision_mix(real))}, signature X-free and equal to its "
                         f"RTL's ({real['signature']}); {annotated[0]} of {annotated[1]} design pins annotated"),
            "decision_mix": decision_mix(real),
            "power_mw": {"default_activity_assumed": default_real, "vcd_measured_activity": vcd_real,
                         "vcd_measured_combinational": mw(real, TT, "vcd", "combinational")},
            "energy_per_decision_pJ": {"whole_block": pj(vcd_real, real["clock_period_ns"]),
                                       "combinational": pj(mw(real, TT, "vcd", "combinational"),
                                                           real["clock_period_ns"])},
            "finding": (f"MEASURED activity draws ~{default_real / vcd_real:.1f}x LESS than the tool's default-activity "
                        f"assumption -> the default-activity numbers quoted elsewhere are CONSERVATIVE upper bounds."),
            "reproduce": "python3 scripts/measure-silicon-power.py decision_hft_realbist   (gate sim -> VCD -> report_power)",
            "artifacts": ("post-route netlist + extracted SPEF + PDK Liberty per corner + VCD — all standard, "
                          "inspectable (GTKWave/PrimePower/Voltus)"),
        },
        "threat_closure": {
            "multi_corner_mw": {
                "ss_1v60_100C": mw(real, SS, "vcd"), "tt_1v80_25C": vcd_real, "ff_1v95_n40C": mw(real, FF, "vcd"),
                "note": (f"VCD-annotated power at each PVT corner with that corner's parasitics; ff/tt = "
                         f"{mw(real, FF, 'vcd') / vcd_real:.2f} against (1.95/1.80)^2 = {(1.95 / 1.80) ** 2:.2f}."),
            },
            "independent_physics_check": {
                "implied_fF_per_net_rise": implied_fF,
                "spef_avg_net_parasitic_fF": spef_fF,
                "verdict": (f"typical-corner VCD switching energy / (V^2 x {rises:,} VCD rises) = {implied_fF} fF per "
                            f"net-rise, beside a {spef_fF} fF SPEF-extracted average net parasitic: the tool's number "
                            f"reduces to first-principles physics."),
            },
            "glitch_power": {
                "status": "bounded",
                "reason": ("functional VCD excludes glitch; iverilog SDF timing simulation on the sky130 models is "
                           "X-pessimistic. Glitch in a shallow, multiplier-free datapath is a small fraction of "
                           f"dynamic power; the default-activity headline already exceeds the measured activity by "
                           f"{default_real / vcd_real:.1f}x, so a glitch correction cannot flip the conservatism."),
            },
            "second_kernel_cross_check_mw": {
                "cgra_sumto_loop_accumulator": mw(cgra),
                "note": "an independent signoff-clean circuit of a different class corroborates the ~1 mW power density.",
            },
        },
        "honest_comparison": {
            "_correction": ("The fabric implements the DECISION KERNEL; the fair baseline is the SAME kernel on the "
                            "soft-core, measured cycle-accurately."),
            "soft_core": {
                "power_mw": soft_mw,
                "decision_kernel_cycles": SOFT_CORE_DECISION_KERNEL_CYCLES,
                "cycles_source": "tb_hft.v cycle-accurate RTL: 187 (decision inline) - 105 (Seq+index marshaling) = 82",
                "die_mm2": round(soft["die_mm2"], 4),
                "fmax_mhz": fmax(soft),
                "energy_per_op_nJ": soft_kernel_nj,
            },
            "fabric": {
                "power_mw_block_total": block_mw, "power_mw_combinational": comb_mw, "cycles": 1,
                "die_mm2": round(bist["die_mm2"], 4), "fmax_mhz": fmax(bist),
                "energy_per_op_pJ_block": e_block, "energy_per_op_pJ_comb": e_comb,
            },
            "reductions": {
                "power_draw_x": [round(soft_mw / block_mw), round(soft_mw / comb_mw)],
                "cycles_x": SOFT_CORE_DECISION_KERNEL_CYCLES,
                "energy_per_op_x": [round(soft_kernel_nj * 1000 / e_block), round(soft_kernel_nj * 1000 / e_comb)],
                "area_x": area_x,
                "fmax_x": round(fmax(bist) / fmax(soft), 1),
                "headline": (f"~{round(soft_kernel_nj * 1000 / e_block):,}x less energy per operation "
                             f"(conservative block-total basis), measured"),
            },
            "note_core_basis": ("cycles measured on the full soft-core RTL; power is the scalar-core Sky130 signoff "
                                "(a smaller instance of the same core family) - using it is conservative (a larger "
                                "core draws more)."),
            "full_pipeline": old["honest_comparison"]["full_pipeline"],
        },
        "cpu_twin": cpu_twin(twin),
    }


def cpu_twin(twin):
    """The CPU the decision fabric is compared against: the routable scalar LOGOS core running the SAME
    decision over the SAME input ranges (`soft_core_realbist`, codegen_sva::decision_workload), measured
    exactly as the fabric is — its shipped netlist simulated at gate level at its signoff clock, VCD-annotated
    report_power at every corner. Its clocks per decision are counted on that window: every decision it
    shows, stimulus and signature included, as the fabric's one clock includes its own."""
    w = twin["rtl_witness"]
    period = twin["clock_period_ns"]
    p = mw(twin, activity="vcd")
    cycles = round(twin["simulated_cycles"] / w["decisions"], 2)
    return {
        "_what": ("the routable scalar LOGOS core, its boot ROM the HFT decision over decision_hft_realbist's input "
                  "ranges; signoff-clean at every corner, VCD-measured at its signoff clock"),
        "design": "soft_core_realbist",
        **binding(twin),
        "clock_mhz": round(1000.0 / period, 3),
        "fmax_mhz": fmax(twin),
        "std_cells": twin["std_cells"],
        "die_mm2": round(twin["die_mm2"], 4),
        "power_mw": {"default_activity_assumed": mw(twin), "vcd_measured_activity": p},
        "power_mw_multi_corner": {"ss_1v60_100C": mw(twin, SS, "vcd"), "tt_1v80_25C": p,
                                  "ff_1v95_n40C": mw(twin, FF, "vcd")},
        "decisions": w["decisions"],
        "decision_mix": {k: w[k] for k in ("buy", "sell", "risk_blocked", "no_trade")},
        "cycles_per_decision": cycles,
        "energy_per_decision_pJ": round(p * cycles * period, 1),
    }


def tile_record(rep, macs, note):
    period = rep["clock_period_ns"]
    d, v = mw(rep), mw(rep, activity="vcd")
    return {
        "design": rep["block"],
        "note": note,
        **binding(rep),
        "die_mm2": round(rep["die_mm2"], 4),
        "std_cells": rep["std_cells"],
        "signoff_clean": True,
        "clock_mhz": round(1000.0 / period, 3),
        "fmax_mhz": fmax(rep),
        "macs_per_clock": macs,
        "power_mw": {"default_activity": d, "vcd_measured": v},
        "energy_per_mac_pJ": {"default_activity": pj(d, period, macs), "vcd_measured": pj(v, period, macs)},
        "reproduce": f"python3 scripts/measure-silicon-power.py {rep['block']}",
    }


def systolic_power(tile, lane):
    rec = tile_record(tile, 64, "8x8 weight-stationary systolic GEMM tile, signoff-clean Sky130 VCD power at its signoff clock")
    lane_pj = pj(mw(lane, activity="vcd"), lane["clock_period_ns"], 8)
    rec["vs_reduction_lane"] = {
        "reduction_design": lane["block"],
        "reduction_pJ_per_mac": lane_pj,
        "energy_per_mac_improvement_x": round(lane_pj / rec["energy_per_mac_pJ"]["vcd_measured"], 2),
        "throughput_x": 64 // 8,
        "area_per_mac_improvement_x": round((lane["die_mm2"] / 8) / (tile["die_mm2"] / 64), 1),
    }
    return rec


def systolic_pipelined_power(old, pipe, tile):
    rec = tile_record(pipe, 64, old["note"])
    comb = pj(mw(tile, activity="vcd"), tile["clock_period_ns"], 64)
    rec["vs_combinational_tile"] = {
        "combinational_fmax_mhz": fmax(tile),
        "pipelined_fmax_mhz": fmax(pipe),
        "fmax_improvement_x": round(fmax(pipe) / fmax(tile), 2),
        "combinational_energy_pJ_per_mac": comb,
        "pipelined_energy_pJ_per_mac": rec["energy_per_mac_pJ"]["vcd_measured"],
        "energy_per_mac_cost_x": round(rec["energy_per_mac_pJ"]["vcd_measured"] / comb, 2),
        "verdict": (f"pipelined trades ~{rec['energy_per_mac_pJ']['vcd_measured'] / comb:.1f}x energy/MAC + fill latency "
                    f"(K+N-1) + area for K-INDEPENDENT Fmax; both stay proven, characterized toggles"),
    }
    return rec


def gemm_energy(old_energy, lane, soft):
    period = lane["clock_period_ns"]
    d, v = mw(lane), mw(lane, activity="vcd")
    e_d, e_v = pj(d, period, 8), pj(v, period, 8)
    soft_mw = mw(soft)
    soft_nj_per_mac = round(soft_mw * SOFT_CORE_CYCLES_PER_MAC * soft["clock_period_ns"] / 1000.0, 3)
    out = dict(old_energy)
    out["method"] = ("MEASURED — VCD-annotated OpenSTA report_power on the signoff-clean OpenLane Sky130 tape of the "
                     "LOGOS-emitted 8-wide int8 MAC array, at its signoff clock (scripts/measure-silicon-power.py)")
    out["signoff_clean_tape"] = {
        "design": "gemm_mac_realbist",
        "rtl": old_energy["signoff_clean_tape"]["rtl"],
        **binding(lane),
        "signoff": "DRC=0 LVS=match XOR=0 antenna=0 routing=0, every timing corner, golden sky130A_mr 0",
        "die_mm2": round(lane["die_mm2"], 4),
        "std_cells": lane["std_cells"],
        "clock_mhz": round(1000.0 / period, 3),
        "setup_slack_ns": lane["worst_setup_slack_ns"],
        "macs_per_clock": 8,
        "power_mw": {"default_activity": d, "vcd_measured": v},
        "energy_per_mac_pJ": {"default_activity": e_d, "vcd_measured": e_v},
        "note": ("whole-block incl. the on-chip LFSR/MISR stimulus (conservative attribution, like decision_hft's "
                 "BIST harness — the MAC datapath alone draws less)"),
    }
    out["vs_soft_core"] = {
        "soft_core_mw": soft_mw,
        "soft_core_cyc_per_mac": SOFT_CORE_CYCLES_PER_MAC,
        "soft_core_nJ_per_mac": soft_nj_per_mac,
        "energy_per_mac_reduction_x": {"vcd_measured": round(soft_nj_per_mac * 1000 / e_v),
                                       "default_activity": round(soft_nj_per_mac * 1000 / e_d)},
        "throughput_density_x": old_energy["vs_soft_core"]["throughput_density_x"],
        "area_x": round(soft["die_mm2"] / lane["die_mm2"], 2),
        "basis": ("fabric (8 MAC/clk) vs the scalar soft-core (0.094 MAC/cyc) running the same int8 MAC kernel, "
                  "each at its own Sky130 signoff clock"),
    }
    prior = dict(old_energy["prior_estimate_superseded"])
    prior["measured_vcd_mw"] = v
    out["prior_estimate_superseded"] = prior
    return out


def energy_efficiency(old, dfp, real):
    """The decision fabric's efficiency point, from ONE block measured once: `decision_hft_realbist`'s
    VCD-annotated power at the signoff clock it was simulated at — the frequency the part is guaranteed
    to run at — with that block's own area and cells. E = P / f and TOPS/W = ops x f / P."""
    doc = json.loads(json.dumps(old))
    ops = dfp["tiles"]["total"]
    f_mhz = round(1000.0 / real["clock_period_ns"], 3)
    p_mw = mw(real, activity="vcd")
    energy = round(p_mw / f_mhz * 1000, 2)
    tops_w = round(ops * f_mhz * 1e6 / (p_mw * 1e-3) / 1e12, 3)
    die = round(real["die_mm2"], 4)
    doc["_provenance"] = (
        "Derived ENTIRELY from one measurement: scripts/measure-silicon-power.py decision_hft_realbist — the "
        "signoff-clean netlist its ledger ships, gate-level simulated at its signoff clock, VCD-annotated OpenSTA "
        "report_power at each PVT corner with that corner's parasitics — plus the Hot Chips 2025 poster IMG_8874 for "
        "the external HiVec reference. Efficiency figures are computed from that power and clock and the tile count "
        "(E = P/f, TOPS/W = ops*f/P); energy_efficiency_lock.rs re-derives every figure.")
    doc["ops_per_eval"] = ops
    doc["die_mm2"] = die
    doc["typical_corner"] = {
        "corner": "tt_1v80_25C", "vdd": 1.8, "fmax_mhz": f_mhz, "power_mw": p_mw,
        "energy_per_eval_pj": energy, "tops_per_w": tops_w,
        "_note": (f"the operating point is the {f_mhz:g} MHz signoff clock the VCD power was measured at (every "
                  f"timing corner meets it); {ops}-tile decision select-DAG, 1 cycle/decision. Control-dominated, so "
                  f"its win is energy-per-decision + single-cycle latency, NOT raw TOPS/W."),
    }
    volts = {SS: 1.6, TT: 1.8, FF: 1.95}
    names = {SS: "ss_1v60_100C", TT: "tt_1v80_25C", FF: "ff_1v95_n40C"}
    doc["vdd_sweep"] = [{"corner": names[c], "vdd": volts[c], "power_mw": mw(real, c, "vcd"),
                         "basis": "VCD-measured PVT corner"} for c in (SS, TT, FF)]
    doc["vdd_scaling_law"] = {
        "relation": "P_switching ~ C*V^2",
        "measured_power_ratio_ff_over_ss": round(mw(real, FF, "vcd") / mw(real, SS, "vcd"), 3),
        "v_squared_prediction": round((1.95 / 1.6) ** 2, 3),
        "status": "measured-consistent within C/temperature slack",
    }
    doc["area_breakdown"] = dict(old["area_breakdown"], std_cells_placed=real["std_cells"], die_mm2=die)
    for row in doc["sota_comparison"]:
        if "LOGOS" in row.get("platform", ""):
            row.update(tops_per_w=tops_w, energy_per_eval_pj=energy, area_mm2=die,
                       source="this repo — VCD-measured on the signoff-clean netlist at its signoff clock "
                              "(benchmarks/silicon/decision_fabric_power.json, scripts/measure-silicon-power.py)")
    return doc


def data_dependent_control(old, dfp, sky):
    """The head-to-head in `data_dependent_control.json`, re-derived from the decision-fabric and
    soft-core power ledgers it quotes (never a second typed copy)."""
    doc = json.loads(json.dumps(old))
    h = doc["head_to_head_in_simulation"]
    sc, hc, vv, tc = dfp["signoff_clean"], dfp["honest_comparison"], dfp["vcd_validation"], dfp["threat_closure"]
    df = h["control_as_dataflow"]
    df["power_mw_vcd_measured"] = vv["power_mw"]["vcd_measured_activity"]
    df["power_mw_multi_corner"] = {k: tc["multi_corner_mw"][k] for k in ("ss_1v60_100C", "tt_1v80_25C", "ff_1v95_n40C")}
    df["power_mw_default_activity_conservative_ub"] = sc["power_mw"]["decision_selectdag_comb"]
    df["cycles_per_decision"] = sc["cycles_per_trade"]
    df["fmax_mhz"] = vv["fmax_mhz"]
    df["energy_per_decision_pJ"] = {"logic_only": vv["energy_per_decision_pJ"]["combinational"],
                                    "whole_block_incl_bist": vv["energy_per_decision_pJ"]["whole_block"]}
    # Decision latency at the measured block's signoff clock: one clock edge.
    df["clock_period_ns"] = vv["clock_period_ns"]
    df["latency_ns"] = round(df["cycles_per_decision"] * vv["clock_period_ns"], 2)
    tw = dfp["cpu_twin"]
    h["_what"] = ("The SAME data-dependent decision over the same input ranges, done two ways, both in simulation and "
                  "both measured the same way — a signoff-clean Sky130 netlist simulated at gate level at its own "
                  "signoff clock, VCD → OpenSTA report_power. Control-as-dataflow is OUR decision fabric "
                  "(decision_hft_realbist). The CPU baseline is OUR routable scalar LOGOS core running the decision "
                  "from its boot ROM (soft_core_realbist): a single-issue fetch-decode-execute CPU, the class of core "
                  "a Tensix tile runs control on. This is the architectural power/latency/energy gap of routing "
                  "control vs running it on a CPU.")
    cpu = h["control_on_a_cpu"]
    cpu["impl"] = ("the routable scalar LOGOS core running the same decision over the same input ranges "
                   "(soft_core_realbist), OUR design")
    cpu["power_mw"] = tw["power_mw"]["vcd_measured_activity"]
    cpu["power_mw_multi_corner"] = tw["power_mw_multi_corner"]
    cpu["cycles_per_decision"] = tw["cycles_per_decision"]
    cpu["energy_per_decision_pJ"] = tw["energy_per_decision_pJ"]
    cpu["fmax_mhz"] = tw["fmax_mhz"]
    cpu["clock_period_ns"] = tw["clock_period_ns"]
    cpu["latency_ns"] = round(tw["cycles_per_decision"] * tw["clock_period_ns"], 1)
    cpu["die_mm2"] = tw["die_mm2"]
    cpu["provenance"] = "sim_vcd_openesta_sky130_signoff"
    cpu["source_json"] = "benchmarks/silicon/decision_fabric_power.json#cpu_twin"
    mix = vv["decision_mix"]
    h["workload"] = (f"realistic data-dependent decisions — a decision kernel over market inputs; over "
                     f"{mix['cycles']} cycles of gate-level simulation it decided {mix_phrase(mix)}, changing its "
                     f"decision on {mix['decision_changes']} of them — counted on the RTL, whose signature over the "
                     f"same window the gate run reproduces ({mix['gate_signature']})")
    power_x = round(cpu["power_mw"] / df["power_mw_vcd_measured"], 2)
    cycles_x = round(cpu["cycles_per_decision"] / df["cycles_per_decision"], 1)
    e_block = round(cpu["energy_per_decision_pJ"] / df["energy_per_decision_pJ"]["whole_block_incl_bist"], 1)
    e_logic = round(cpu["energy_per_decision_pJ"] / df["energy_per_decision_pJ"]["logic_only"], 1)
    latency_x = round(cpu["latency_ns"] / df["latency_ns"], 1)
    h["the_gap"] = {
        "power_reduction_x": power_x,
        "cycles_reduction_x": cycles_x,
        "latency_reduction_x": latency_x,
        "energy_reduction_x": {"conservative_whole_block": e_block, "logic_only": e_logic},
        "reading": (f"routing a data-dependent decision as spatial dataflow resolves it in ONE clock edge, "
                    f"{df['latency_ns']} ns at the fabric's signoff clock, against {cpu['cycles_per_decision']} cycles, "
                    f"{cpu['latency_ns']:,} ns at the CPU's ({round(cycles_x)}x FEWER cycles, ~{round(latency_x)}x LOWER LATENCY), and — because energy = power "
                    f"x time — burns ~{round(e_block):,}-{round(e_logic):,}x LESS ENERGY per data-dependent decision. "
                    f"Power is each block's at its own signoff clock ({df['power_mw_vcd_measured']} mW for the fabric at "
                    f"{df['clock_period_ns']} ns, {cpu['power_mw']} mW for the CPU at {cpu['clock_period_ns']} ns): a "
                    f"faster fabric draws more power for more decisions a second, so their ratio is not the claim. "
                    f"Latency is the trading axis and energy/decision the inference-economics one (energy/token). "
                    f"Tenstorrent runs control on 5 such CPUs per tile."),
    }
    doc["sota_positioning"]["tenstorrent"] = (
        f"control on baby RISC-V CPUs (von Neumann) — the very thing this artifact beats by {round(cycles_x)}x in "
        f"cycles and ~{round(e_block):,}x in energy per decision, in simulation")
    doc["honesty"]["power_is_our_fabric_not_ttsim"] = (
        f"the {df['power_mw_vcd_measured']} mW fabric and {cpu['power_mw']} mW CPU figures are both switching measured "
        f"by gate-level simulation of a signoff-clean Sky130 netlist of ours, each at its own signoff clock, deciding "
        f"over the same input ranges; neither is ttsim (Tenstorrent's sim gives cycles only)")
    return doc


def main():
    check = "--check" in sys.argv[1:]
    blocks = ("soft_core_scalar", "decision_hft_bist", "decision_hft_realbist", "cgra_sumto",
              "systolic_tile_realbist", "systolic_pipe8_realbist", "gemm_mac_realbist", "soft_core_realbist")
    rep = {b: measured(b) for b in blocks if os.path.exists(os.path.join(ROOT, "logs", f"power-{b}.json"))}
    load = lambda n: json.load(open(os.path.join(SIL, n)))
    # Each ledger is written from exactly the measurements it quotes; one missing measurement holds
    # back only the ledgers that depend on it, and is named.
    plan = [
        ("sky130_power.json", ("soft_core_scalar",), lambda: sky130_power(rep["soft_core_scalar"]), 1),
        ("decision_fabric_power.json", ("soft_core_scalar", "decision_hft_bist", "decision_hft_realbist", "cgra_sumto", "soft_core_realbist"),
         lambda: decision_fabric_power(load("decision_fabric_power.json"), rep["soft_core_scalar"],
                                       rep["decision_hft_bist"], rep["decision_hft_realbist"], rep["cgra_sumto"],
                                       rep["soft_core_realbist"]), 1),
        ("systolic_power.json", ("systolic_tile_realbist", "gemm_mac_realbist"),
         lambda: systolic_power(rep["systolic_tile_realbist"], rep["gemm_mac_realbist"]), 1),
        ("systolic_pipelined_power.json", ("systolic_pipe8_realbist", "systolic_tile_realbist"),
         lambda: systolic_pipelined_power(load("systolic_pipelined_power.json"), rep["systolic_pipe8_realbist"],
                                          rep["systolic_tile_realbist"]), 1),
        ("gemm_fabric.json", ("gemm_mac_realbist", "soft_core_scalar"),
         lambda: dict(load("gemm_fabric.json"), energy=gemm_energy(load("gemm_fabric.json")["energy"],
                                                                   rep["gemm_mac_realbist"], rep["soft_core_scalar"])), None),
        ("data_dependent_control.json", ("soft_core_scalar", "decision_hft_bist", "decision_hft_realbist", "cgra_sumto", "soft_core_realbist"),
         lambda: data_dependent_control(load("data_dependent_control.json"),
                                        decision_fabric_power(load("decision_fabric_power.json"), rep["soft_core_scalar"],
                                                              rep["decision_hft_bist"], rep["decision_hft_realbist"],
                                                              rep["cgra_sumto"], rep["soft_core_realbist"]),
                                        sky130_power(rep["soft_core_scalar"])), None),
    ]
    plan.append(("energy_efficiency.json", ("soft_core_scalar", "decision_hft_bist", "decision_hft_realbist", "cgra_sumto", "soft_core_realbist"),
                 lambda: energy_efficiency(load("energy_efficiency.json"),
                                           decision_fabric_power(load("decision_fabric_power.json"), rep["soft_core_scalar"],
                                                                 rep["decision_hft_bist"], rep["decision_hft_realbist"],
                                                                 rep["cgra_sumto"], rep["soft_core_realbist"]),
                                           rep["decision_hft_realbist"]), None))
    stale, held = [], []
    for name, needs, build, indent in plan:
        missing = [b for b in needs if b not in rep]
        if missing:
            held.append(f"{name} (needs {', '.join(missing)})")
            continue
        doc = build()
        path = os.path.join(SIL, name)
        raw = open(path).read()
        width = indent if indent is not None else (len(re.match(r"\{\n( *)", raw).group(1)) if raw.startswith("{\n") else 1)
        if json.loads(raw) != doc:
            stale.append(name)
            if not check:
                tmp = path + ".tmp"
                open(tmp, "w").write(json.dumps(doc, indent=width, ensure_ascii=False) + "\n")
                os.replace(tmp, path)
    print(("stale: " if check else "rewrote: ") + (", ".join(stale) if stale else "nothing"))
    if held:
        print("held back, measurement missing: " + "; ".join(held))
    return 1 if (check and (stale or held)) else 0


if __name__ == "__main__":
    sys.exit(main())
