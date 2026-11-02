#!/usr/bin/env python3
"""Extract the FULL signoff evidence for a taped-out block from its OpenLane run directory and
merge it into benchmarks/silicon/sky130_signoff.json.

The top-line gates (DRC / LVS / antenna / routing) are necessary but not sufficient to call a
result "signoff-clean" the way a foundry would. This recorder also pulls the DEEP evidence a
skeptical reviewer (or a golden signoff deck) checks:

  • setup AND hold worst slack, plus the count of VIOLATED timing paths (post-RCX, multi-corner),
  • the KLayout-vs-Magic GDS XOR difference count (catches dropped/extra geometry),
  • the explicit antenna-violators list length (not just the summary counter),
  • the Netgen "LVS clean / unique match" verdict, not just the error counter,
  • the worst VPWR IR-drop in millivolts,
  • whether multi-corner STA actually ran.

Every field is fail-CLOSED: a missing report yields a sentinel that makes the lock go RED, never a
silent pass. The reports live under a run dir that is NOT checked in; this script distills them into
the checked-in JSON the Rust locks read.

Usage:  record-sky130-signoff.py <run_dir> <block> <out.json>
        record-sky130-signoff.py --verify <ledger.json> [run_root]

The --verify mode is the inverse: instead of writing a record it RE-DERIVES every block already in
a ledger from the run directories under `run_root` (default `tools/OpenLane/designs`) and reports
any field that no longer matches. A recorded signoff claim whose run directory is gone or has been
overwritten by a failing re-sign is otherwise undetectable — the locks all read the ledger, so a
dead run's numbers stay green forever. This mode is what `sky130_provenance_lock` calls. It reuses
the extractors above verbatim, so the verifier and the recorder can never disagree by construction.
"""
import csv
import fcntl
import json
import os
import re
import sys


def _first(glob_dir, suffix):
    """The first file in glob_dir whose name ends with suffix (or None)."""
    if not os.path.isdir(glob_dir):
        return None
    for name in sorted(os.listdir(glob_dir)):
        if name.endswith(suffix):
            return os.path.join(glob_dir, name)
    return None


def _last_indexed(glob_dir, suffix):
    """The highest step-numbered file ending with suffix. OpenLane names step reports '<N>-<name>'; the antenna
    ECO writes its intermediate check reports into the signoff dir BEFORE the final signoff antenna check, so
    the authoritative (post-jog) report is the highest-numbered one — not the alphabetically-first stale one
    ('28-...' sorts before '46-...'). Numeric, not lexical, so '100-' beats '28-'."""
    if not os.path.isdir(glob_dir):
        return None
    cands = [n for n in os.listdir(glob_dir) if n.endswith(suffix)]
    if not cands:
        return None

    def _idx(n):
        m = re.match(r"(\d+)", n)
        return int(m.group(1)) if m else -1

    return os.path.join(glob_dir, max(cands, key=_idx))


RC_CORNERS = ("min", "nom", "max")


def _mca_dirs(signoff):
    """The multi-corner STA directories (`NN-mca/`), where OpenLane writes one report set per RC
    corner, each covering the Slowest / Typical / Fastest library corners."""
    if not os.path.isdir(signoff):
        return []
    return [os.path.join(signoff, n) for n in sorted(os.listdir(signoff))
            if "mca" in n and os.path.isdir(os.path.join(signoff, n))]


def _sta_reports(signoff, kind):
    """EVERY post-RCX STA report of one kind (`summary`, `max`, `min`, `checks`): the typical-corner
    `NN-rcx_sta.<kind>.rpt` at the top level AND each multi-corner `NN-mca/rcx_<rc>_sta.<kind>.rpt`.

    Timing is read worst-case across all of them. Reading only the top level is what let a slow
    corner at -121.40 ns stand behind a recorded +52.46 ns (fabric_soc, 2026-09-23): the multi-
    corner reports are one directory down and named per RC corner, so a flat `*rcx_sta.<kind>.rpt`
    glob matched none of them."""
    if not os.path.isdir(signoff):
        return []
    out = [os.path.join(signoff, n) for n in sorted(os.listdir(signoff))
           if n.endswith(f"rcx_sta.{kind}.rpt")]
    for d in _mca_dirs(signoff):
        out += [os.path.join(d, n) for n in sorted(os.listdir(d))
                if re.fullmatch(rf"rcx_({'|'.join(RC_CORNERS)})_sta\.{kind}\.rpt", n)]
    return out


def _read(path):
    try:
        with open(path, encoding="utf-8", errors="replace") as fh:
            return fh.read()
    except (OSError, TypeError):
        return None


def xor_differences(signoff):
    txt = _read(_first(signoff, "xor.rpt"))
    if txt is None:
        return -1  # sentinel → lock RED
    m = re.search(r"Total XOR differences\s*=\s*(\d+)", txt)
    return int(m.group(1)) if m else -1


def sta_slacks(signoff):
    """(setup_worst_ns, hold_worst_ns) = the WORST (minimum) slack across EVERY post-RCX STA summary
    report — so a failing slow corner can never be masked by an optimistic nominal one. None on absence
    (→ the lock goes RED, fail-closed)."""
    out = {"setup_worst_slack_ns": None, "hold_worst_slack_ns": None}
    for path in _sta_reports(signoff, "summary"):
        txt = _read(path)
        if txt is None:
            continue
        # The summary prints a "-max (Setup)" then a later "worst slack X", likewise "-min (Hold)".
        for key, tag in (("setup_worst_slack_ns", "-max (Setup)"), ("hold_worst_slack_ns", "-min (Hold)")):
            idx = txt.find(tag)
            if idx != -1:
                m = re.search(r"worst slack\s+(-?\d+(?:\.\d+)?)", txt[idx:])
                if m:
                    v = float(m.group(1))
                    out[key] = v if out[key] is None else min(out[key], v)
    return out


def violated_paths(signoff):
    """Count of VIOLATED timing paths in the setup(max)/hold(min) post-RCX reports — the WORST (maximum)
    across every corner report, fail-closed: a missing OR unreadable report yields the -1 sentinel so the
    lock goes RED rather than silently reading 0 violations."""
    def count(kind):
        paths = _sta_reports(signoff, kind)
        if not paths:
            return -1  # sentinel → lock RED
        worst = -1
        for p in paths:
            txt = _read(p)
            if txt is None:
                continue
            n = len(re.findall(r"\(VIOLATED\)|slack \(VIOLATED\)|VIOLATED", txt))
            worst = max(worst, n)
        return worst  # -1 if every report was unreadable → RED
    return count("max"), count("min")


def antenna_violators(signoff):
    """Number of listed antenna violators in the FINAL signoff antenna check (0 == the file exists and is
    empty). Must read the last-indexed report: the antenna ECO leaves earlier, pre-repair reports in the dir."""
    path = _last_indexed(signoff, "antenna_violators.rpt")
    if path is None:
        return -1
    txt = _read(path) or ""
    return len([ln for ln in txt.splitlines() if ln.strip()])


def lvs_clean(signoff):
    txt = _read(_first(signoff, "lvs.rpt"))
    if txt is None:
        return False
    return ("no net, device, pin, or property mismatches" in txt
            or "Design is LVS clean" in txt
            or re.search(r"Total errors\s*=\s*0", txt) is not None)


def ir_drop_worst_mv(signoff, nominal_v=1.8, vdd="VPWR"):
    """Worst IR-drop on the block's supply `vdd` in mV = (nominal - min instance voltage) * 1000. The
    supply is the run's own first `VDD_NETS` entry: `VPWR` for a standalone block, the harness rail
    (`vccd1`) for a Caravel user macro."""
    path = _first(signoff, f"irdrop-{vdd}.rpt")
    txt = _read(path)
    if txt is None:
        return -1.0
    volts = []
    for row in csv.reader(txt.splitlines()):
        if not row:
            continue
        try:
            volts.append(float(row[-1]))
        except (ValueError, IndexError):
            continue
    if not volts:
        return -1.0
    return round((nominal_v - min(volts)) * 1000.0, 4)


def multi_corner(signoff):
    """True only when multi-corner STA COMPLETED: every RC corner (min / nom / max) left a readable
    summary carrying a setup worst slack. A directory merely existing proves nothing — an aborted
    run leaves one too."""
    done = set()
    for d in _mca_dirs(signoff):
        for rc in RC_CORNERS:
            txt = _read(os.path.join(d, f"rcx_{rc}_sta.summary.rpt"))
            if txt and "-max (Setup)" in txt and re.search(r"worst slack\s+-?\d", txt):
                done.add(rc)
    return done == set(RC_CORNERS)


def check_types(signoff):
    """The `report_check_types` violation counters, WORST-CASE across every corner report.

    OpenSTA prints three explicit totals at the tail of each `*rcx_sta.checks.rpt`:
        max slew violation count <n>
        max fanout violation count <n>
        max cap violation count <n>
    A net driving far more load than its cell is characterized for has transition times outside the
    library's model, so a slack computed over it is optimistic by an unknown amount. Recording these
    is what makes them fixable — `fabric_soc` shipped `signoff_clean: true` carrying 4,929 max-fanout
    violations precisely because no field held the number.

    Fail-CLOSED: no report, or a report missing a counter, yields -1 (a sentinel the lock rejects),
    never a silent 0.
    """
    out = {"max_slew_violations": -1, "max_fanout_violations": -1, "max_cap_violations": -1}
    keys = {
        "max_slew_violations": r"max slew violation count\s+(\d+)",
        "max_fanout_violations": r"max fanout violation count\s+(\d+)",
        "max_cap_violations": r"max cap violation count\s+(\d+)",
    }
    reports = _sta_reports(signoff, "checks")
    if not reports:
        return out
    for field, pat in keys.items():
        worst = None
        for path in reports:
            txt = _read(path)
            if txt is None:
                continue
            for m in re.finditer(pat, txt):
                n = int(m.group(1))
                worst = n if worst is None else max(worst, n)
        if worst is not None:
            out[field] = worst
    return out


_VERILOG_KEYWORDS = {"module", "endmodule", "input", "output", "inout", "wire", "reg", "assign", "supply0", "supply1"}


def _unescape(name):
    """One spelling for an instance name: Verilog escapes it with a leading backslash, DEF with a
    backslash before each bracket."""
    return name.replace("\\", "").strip()


def _netlist_instances(path):
    """Every instance name a gate-level Verilog netlist declares (`<master> <name> (`)."""
    txt = _read(path)
    if txt is None:
        return None
    return {_unescape(m.group(2)) for m in re.finditer(r"^\s+([A-Za-z_][\w$]*)\s+(\\\S+|[A-Za-z_][\w$\[\].]*)\s*\(", txt, re.M)
            if m.group(1) not in _VERILOG_KEYWORDS}


def _def_components(path):
    """Every component name in a DEF's COMPONENTS section."""
    txt = _read(path)
    if txt is None:
        return None
    m = re.search(r"^COMPONENTS\s+\d+\s*;(.*?)^END COMPONENTS", txt, re.M | re.S)
    if m is None:
        return None
    return {_unescape(n) for n in re.findall(r"^\s*-\s+(\S+)\s", m.group(1), re.M)}


def sta_netlist_cells(run_dir, block):
    """Does the netlist signoff STA read describe the layout that was signed off?

    Returns (missing, extra): the final layout's components absent from the netlist that ANY
    signoff STA run read, and that netlist's instances absent from the layout — worst over every
    STA log in `logs/signoff`. Both 0 means timing, slew and capacitance were judged on the chip
    that was drawn. -1 for both when no STA log names its netlist or a file cannot be read.

    MEASURED: the antenna ECO adds diode and filler cells after detailed routing wrote the netlist,
    and before this was checked the STA kept reading the pre-ECO one — wpe_user_project
    RUN_2026.09.27_18.14.14 signed off 253 ECO diodes it never saw; coproc_core 29,
    fabric_horner_seq 11, infinichip 6 and systolic_pipe16_realbist 3 were recorded clean the
    same way."""
    layout = _def_components(os.path.join(run_dir, "results", "final", "def", block + ".def"))
    logs = os.path.join(run_dir, "logs", "signoff")
    if layout is None or not os.path.isdir(logs):
        return -1, -1
    run = os.path.basename(os.path.normpath(run_dir))
    netlists = set()
    for name in sorted(os.listdir(logs)):
        if name.endswith(".log") and "sta" in name:
            for path in re.findall(r"Reading netlist '([^']+)'", _read(os.path.join(logs, name)) or ""):
                tail = path.split(f"/runs/{run}/", 1)
                netlists.add(os.path.join(run_dir, tail[1]) if len(tail) == 2 else path)
    if not netlists:
        return -1, -1
    missing = extra = 0
    for path in sorted(netlists):
        cells = _netlist_instances(path)
        if cells is None:
            return -1, -1
        missing = max(missing, len(layout - cells))
        extra = max(extra, len(cells - layout))
    return missing, extra


def _tcl_env(run_dir):
    """Every `set ::env(KEY) "VALUE"` in the run's own config.tcl — the authoritative record of what
    the flow was actually configured with, written BY the run rather than asserted about it."""
    txt = _read(os.path.join(run_dir, "config.tcl"))
    if txt is None:
        return {}
    return dict(re.findall(r'set \s*::env\((\w+)\)\s+"([^"]*)"', txt))


def constraints(run_dir):
    """The limits the counters above were measured against. A count without its limit cannot be
    compared between blocks, and a block that quietly loosened its limit would otherwise read as
    improved — `fabric_soc`'s 4,929 is against a MAX_FANOUT_CONSTRAINT of 6, which is the whole
    story of that number."""
    env = _tcl_env(run_dir)

    def f(key, default):
        try:
            return float(env.get(key, default))
        except (TypeError, ValueError):
            return -1.0

    return {
        "clock_period_ns": f("CLOCK_PERIOD", -1.0),
        "max_transition_constraint": f("MAX_TRANSITION_CONSTRAINT", -1.0),
        "max_fanout_constraint": f("MAX_FANOUT_CONSTRAINT", -1.0),
        "max_cap_constraint": f("MAX_CAPACITANCE_CONSTRAINT", -1.0),
    }


def pdk(run_dir):
    """The PDK and standard-cell library this run was built against, read from the run's own
    config.tcl rather than hardcoded. The ledger keys blocks by NAME alone, so without this a
    second-PDK harden of the same block silently OVERWRITES the first record."""
    env = _tcl_env(run_dir)
    return {
        "pdk": env.get("PDK", ""),
        "std_cell_library": env.get("STD_CELL_LIBRARY", ""),
    }


def openlane_image(run_dir):
    """The OpenLane image the run flowed in, as its launcher wrote it beside the run (`<run>/openlane_image`), or None
    when the launcher named none and the run flowed in the image its tree's checkout names. The run cannot say itself:
    the bench's image (scripts/openroad/) rebuilds the same OpenROAD commit, which prints the same version line."""
    txt = _read(os.path.join(run_dir, "openlane_image"))
    return txt.strip() if txt and txt.strip() else None


def top_line(run_dir, block):
    """The original gates from metrics.csv + the streamed GDS size."""
    metrics = os.path.join(run_dir, "reports", "metrics.csv")
    rows = list(csv.DictReader(open(metrics)))
    r = rows[0] if rows else {}
    gds = os.path.join(run_dir, "results", "final", "gds", block + ".gds")

    def i(k):
        try:
            return int(float(r.get(k) or 0))
        except ValueError:
            return 0

    def f(k):
        # Rounded to 12 SIGNIFICANT digits: OpenLane writes binary-float noise into metrics.csv
        # (rev_alu_core's critical path read 4.140000000000001), and every document that recites a
        # ledger number would otherwise have to recite the noise. Significant digits, not decimal
        # places: a die area of 0.037064104425 mm^2 is real precision, and rounding it to 6 places
        # moved it 4e-7 — enough for the provenance verifier (1e-9) to reject 16 true records.
        try:
            return float(f"{float(r.get(k) or 0):.12g}")
        except ValueError:
            return 0.0

    return {
        "flow_status": r.get("flow_status"),
        "gds_bytes": os.path.getsize(gds) if os.path.exists(gds) else 0,
        "die_area_mm2": f("DIEAREA_mm^2"),
        "core_area_um2": f("CoreArea_um^2"),
        "std_cells": i("TotalCells"),
        "drc_violations": i("Magic_violations"),
        "lvs_errors": i("lvs_total_errors"),
        "antenna_violations": i("pin_antenna_violations") + i("net_antenna_violations"),
        "routing_violations": i("tritonRoute_violations"),
        "critical_path_ns": f("critical_path_ns"),
        "wns_ns": f("wns"),
    }


# Cells that carry no logic: well taps, decoupling capacitors, fill, and antenna diodes. Their number is set by
# the floorplan and the antenna repair, not by the design.
PHYSICAL_ONLY = re.compile(r"__(tapvpwrvgnd|decap|fill|diode|fakediode)_")


def _liberty_areas(path):
    """Every cell's area in a Liberty file, by name (None when it cannot be read)."""
    txt = _read(path) if path else None
    if txt is None:
        return None
    return {m.group(1): float(m.group(2))
            for m in re.finditer(r'\bcell\s*\(\s*"?([\w$]+)"?\s*\)\s*\{.*?\barea\s*:\s*([0-9.]+)', txt, re.S)}


def std_cell_area(run_dir, block):
    """The area the design's logic occupies on the die: the Liberty area of every instance in the final
    netlist, the physical-only cells left out. The die and core areas are set by FP_CORE_UTIL and cannot be
    compared between designs; this can. -1 when the netlist, the library or a cell's area cannot be read."""
    lib = _liberty_areas(_tcl_env(run_dir).get("LIB_SYNTH_COMPLETE"))
    txt = _read(os.path.join(run_dir, "results", "final", "verilog", "gl", block + ".v"))
    if lib is None or txt is None:
        return -1.0
    total = 0.0
    for master in re.findall(r"^\s+(sky130_\w+)\s+\S+\s*\(", txt, re.M):
        if PHYSICAL_ONLY.search(master):
            continue
        if master not in lib:
            return -1.0
        total += lib[master]
    return round(total, 6)


def synth_areas(run_dir):
    """Every module's area as synthesis reported it (yosys `stat -liberty`, the last synthesis pass), by name. A
    module kept whole under `keep_hierarchy` — a comparative candidate inside its fold wrapper — reports its
    own area apart from the wrapper's; the top's is the whole design's. Empty when there is no report."""
    rpt = _last_indexed(os.path.join(run_dir, "reports", "synthesis"), ".stat.rpt")
    txt = _read(rpt) if rpt else None
    if txt is None:
        return {}
    return {m: round(float(a), 6) for m, a in re.findall(r"Chip area for module '\\?([^']+)':\s*([0-9.]+)", txt)}


def synth_findings(run_dir):
    """What OpenLane's synthesis checks find in the run, by kind — the checks a signed-off block's flow stops on and a
    comparative one's records (ClosureSettings::comparative). From yosys's CHECK pass before synthesis
    (`*_pre_synth.chk.rpt`): wires used without a driver, conflicting drivers (a tristate bus excepted, as OpenLane
    excepts it), logic loops. From the synthesis log: latches, range selects out of bounds, ports resized to fit their
    signals. Each count is of findings, every bit a finding."""
    out = {"undriven": 0, "conflicting_drivers": 0, "logic_loops": 0, "latches": 0, "out_of_bounds": 0,
           "port_resizing": 0}
    rpt = _first(os.path.join(run_dir, "reports", "synthesis"), "_pre_synth.chk.rpt")
    txt = _read(rpt) if rpt else None
    if txt:
        out["undriven"] = len(re.findall(r"^Warning: Wire .* is used but has no driver\.", txt, re.M))
        for warning in re.split(r"^(?=Warning: )", txt, flags=re.M):
            if warning.startswith("Warning: multiple conflicting drivers") and "($tribuf)" not in warning:
                out["conflicting_drivers"] += 1
            elif warning.startswith("Warning: found logic loop"):
                out["logic_loops"] += 1
    log = _first(os.path.join(run_dir, "logs", "synthesis"), "synthesis.log")
    txt = _read(log) if log else None
    if txt:
        out["latches"] = len(re.findall(r"\$LATCH", txt))
        out["out_of_bounds"] = len(re.findall(r"out of bounds on signal", txt))
        out["port_resizing"] = len(re.findall(r"Resizing cell port", txt))
    return out


def extract(run_dir, block):
    signoff = os.path.join(run_dir, "reports", "signoff")
    sig = top_line(run_dir, block)
    sig.update(sta_slacks(signoff))
    sv, hv = violated_paths(signoff)
    sig["setup_violated_paths"] = sv
    sig["hold_violated_paths"] = hv
    sig["xor_differences"] = xor_differences(signoff)
    sig["antenna_violators"] = antenna_violators(signoff)
    sig["lvs_clean"] = lvs_clean(signoff)
    sig["ir_drop_worst_mv"] = ir_drop_worst_mv(signoff, vdd=(_tcl_env(run_dir).get("VDD_NETS") or "VPWR").split()[0])
    sig["multi_corner"] = multi_corner(signoff)
    sig.update(check_types(signoff))
    sig["sta_netlist_missing_cells"], sig["sta_netlist_extra_cells"] = sta_netlist_cells(run_dir, block)
    sig.update(constraints(run_dir))
    sig.update(pdk(run_dir))
    image = openlane_image(run_dir)
    if image is not None:
        sig["openlane_image"] = image
    sig["std_cell_area_um2"] = std_cell_area(run_dir, block)
    modules = synth_areas(run_dir)
    sig["synth_area_um2"] = modules.get(block, -1.0)
    sig["synth_module_areas_um2"] = modules
    sig["synth_findings"] = synth_findings(run_dir)

    setup = sig["setup_worst_slack_ns"]
    hold = sig["hold_worst_slack_ns"]
    # signoff_clean is the CONJUNCTION of every gate — top-line AND deep. Fail-closed throughout.
    sig["signoff_clean"] = bool(
        sig["flow_status"] in ("flow completed", "flow_completed")
        and sig["gds_bytes"] > 0
        and sig["std_cells"] > 0
        and sig["drc_violations"] == 0
        and sig["lvs_errors"] == 0
        and sig["lvs_clean"]
        and sig["antenna_violations"] == 0
        and sig["antenna_violators"] == 0
        and sig["routing_violations"] == 0
        and sig["xor_differences"] == 0
        and setup is not None and setup > 0
        and hold is not None and hold > 0
        and sig["setup_violated_paths"] == 0
        and sig["hold_violated_paths"] == 0
        and 0 <= sig["ir_drop_worst_mv"] < 90.0
        and sig["multi_corner"]
        # ELECTRICAL. slew and capacitance are the physically meaningful checks and the ones the
        # resizer can actually repair (`repair_design -slew_margin/-cap_margin`).
        and sig["max_slew_violations"] == 0
        and sig["max_cap_violations"] == 0
        # Every number above was computed on the netlist the STA read; it must be the layout's.
        and sig["sta_netlist_missing_cells"] == 0
        and sig["sta_netlist_extra_cells"] == 0
    )
    # max_FANOUT is recorded but deliberately NOT gated, and the evidence for that is specific.
    # There is no `-max_fanout` anywhere in OpenLane's scripts: `repair_design` is only ever given
    # -max_wire_length/-slew_margin/-cap_margin, so fanout is not repairable at P&R at all — it
    # reaches only abc's `buffer -c -N` at synthesis and `set_max_fanout` in base.sdc, which
    # REPORTS. And it is routinely reported against things the constraint does not govern: on the
    # Caravel wrapper, ALL 96 "violations" are CTS clock buffers inside the macro
    # (mprj/clkbuf_*_wb_clk_i/X) driving 26-40 sinks against a limit of 10, in a design containing
    # ONE cell, with slew 0 and cap 0. A clock buffer driving 40 flops is correct CTS behaviour;
    # `set_max_fanout` is the wrong constraint for a clock tree, and the physical checks it stands
    # in for are both clean. Gating on it would fail a design for a misapplied constraint.
    return sig


RUN_ROOT = os.path.join("tools", "OpenLane", "designs")

# Fields that are re-derived from the run directory. Anything the recorder writes is re-checkable;
# comparing the full set means no gate can be quietly dropped from the comparison later.
VERIFY_FIELDS = (
    "flow_status", "gds_bytes", "die_area_mm2", "core_area_um2", "std_cells",
    "drc_violations", "lvs_errors", "antenna_violations", "routing_violations",
    "critical_path_ns", "wns_ns", "setup_worst_slack_ns", "hold_worst_slack_ns",
    "setup_violated_paths", "hold_violated_paths", "xor_differences", "antenna_violators",
    "lvs_clean", "ir_drop_worst_mv", "multi_corner",
    "max_slew_violations", "max_fanout_violations", "max_cap_violations",
    "sta_netlist_missing_cells", "sta_netlist_extra_cells",
    "clock_period_ns", "max_transition_constraint", "max_fanout_constraint", "max_cap_constraint",
    "pdk", "std_cell_library", "openlane_image",
    "std_cell_area_um2", "synth_area_um2", "synth_module_areas_um2", "synth_findings",
    "signoff_clean",
)


def _same(a, b):
    """Field equality, tolerant only of float representation — never of a differing value."""
    if isinstance(a, bool) or isinstance(b, bool):
        return bool(a) == bool(b)
    if isinstance(a, (int, float)) and isinstance(b, (int, float)):
        return abs(float(a) - float(b)) <= 1e-9
    return a == b


def design_of(run_dir):
    """The OpenLane design directory a run belongs to: `<run_root>/<design>/runs/<run>`."""
    return os.path.basename(os.path.dirname(os.path.dirname(os.path.abspath(run_dir.rstrip("/")))))


def run_dirs(run_root, block, design=None):
    """Every OpenLane run directory for a block, oldest first. A block may have been re-signed
    several times; ANY run that fully reproduces the record vindicates it. The runs live under
    `logos_<block>` unless the record names another design directory."""
    d = os.path.join(run_root, design or "logos_" + block, "runs")
    if not os.path.isdir(d):
        return []
    return [os.path.join(d, n) for n in sorted(os.listdir(d)) if os.path.isdir(os.path.join(d, n))]


def compare(recorded, live):
    """The per-field deltas between a recorded entry and a freshly extracted one."""
    out = {}
    for k in VERIFY_FIELDS:
        if k not in recorded:
            continue
        if not _same(recorded[k], live.get(k)):
            out[k] = {"ledger": recorded[k], "disk": live.get(k)}
    return out


def verify_ledger(ledger, run_root):
    """Re-derive every block in a ledger. A block with no run directory is SKIPPED (a build product
    absent from a fresh clone proves nothing); a block whose runs all disagree is a MISMATCH, and the
    deltas reported are those of the most recent run — the one that overwrote the claim. A registered ledger
    nothing has written yet re-derives no block, and says so (sky130_ledger_registry_lock holds it to existing)."""
    report = {"ledger": ledger, "run_root": run_root, "blocks": {},
              "reproduced": 0, "mismatched": 0, "skipped": 0, "missing": not os.path.exists(ledger)}
    if report["missing"]:
        return report
    doc = json.load(open(ledger))

    for block, recorded in doc.get("blocks", {}).items():
        runs = run_dirs(run_root, block, recorded.get("design"))
        if not runs:
            report["blocks"][block] = {"status": "no_run_dir"}
            report["skipped"] += 1
            continue

        newest, deltas = runs[-1], None
        for rd in reversed(runs):
            try:
                d = compare(recorded, extract(rd, block))
            except (OSError, ValueError, KeyError, IndexError) as exc:
                d = {"_extract_error": {"ledger": "(extractable)", "disk": str(exc)}}
            if not d:
                report["blocks"][block] = {"status": "reproduces", "run_dir": rd}
                report["reproduced"] += 1
                break
            if deltas is None:
                deltas = d
        else:
            report["blocks"][block] = {"status": "mismatch", "run_dir": newest, "mismatches": deltas}
            report["mismatched"] += 1

    return report


def run_verify(ledger, run_root):
    report = verify_ledger(ledger, run_root)
    json.dump(report, sys.stdout, indent=2, sort_keys=True)
    sys.stdout.write("\n")

    name = os.path.basename(ledger)
    print(f"== verify {name}: {report['reproduced']} reproduced, "
          f"{report['mismatched']} MISMATCHED, {report['skipped']} skipped (no run dir) ==",
          file=sys.stderr)
    for block, b in sorted(report["blocks"].items()):
        if b["status"] != "mismatch":
            continue
        print(f"  {block}  ({b['run_dir']})", file=sys.stderr)
        for field, d in sorted(b["mismatches"].items()):
            print(f"    {field}: ledger={d['ledger']}  disk={d['disk']}", file=sys.stderr)
    return 0 if report["mismatched"] == 0 else 1


# One lock for every block ledger: flows that finish at once (the comparative bench runs several side by side) record
# one at a time.
LEDGER_LOCK = os.path.join(os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")), "logs",
                           "sky130-ledgers.lock")


def write_record(out, block, sig):
    """Put `sig` into the ledger `out` as `block`'s record, under LEDGER_LOCK: read the ledger, set the record, and
    replace the file whole, so no record another flow wrote meanwhile is lost and no reader sees half a file."""
    os.makedirs(os.path.dirname(LEDGER_LOCK), exist_ok=True)
    with open(LEDGER_LOCK, "a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        try:
            doc = {}
            if os.path.exists(out):
                try:
                    doc = json.load(open(out))
                except ValueError:
                    doc = {}
            # The PDK is read from the RUN's own config.tcl, never asserted here. A hardcoded literal
            # survives a re-harden against a different PDK and turns the ledger into a lie; worse, `blocks`
            # is keyed by block NAME alone, so a second-PDK harden of the same block would overwrite the
            # first record under a meta line still naming the first PDK. Each block also carries its own
            # `pdk`/`std_cell_library`, so a mixed-PDK ledger stays readable.
            doc.setdefault("blocks", {})[block] = sig
            _pdks = sorted({b.get("pdk", "") for b in doc["blocks"].values() if b.get("pdk")})
            doc.setdefault("meta", {})["pdk"] = (
                f"{'/'.join(_pdks)} (open_pdks via ciel)" if _pdks else "(unrecorded)"
            )
            doc["meta"]["flow"] = "OpenLane v1 (yosys + OpenROAD + Magic + Netgen + KLayout)"
            tmp = os.path.join(os.path.dirname(out) or ".", ".%s.%d.tmp" % (os.path.basename(out), os.getpid()))
            with open(tmp, "w") as f:
                json.dump(doc, f, indent=2)
                f.write("\n")
            os.replace(tmp, out)
        finally:
            fcntl.flock(lock, fcntl.LOCK_UN)


ROOT = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
REGISTRY = os.path.join(ROOT, "benchmarks", "silicon", "sky130_ledgers.json")


def registered_as(out):
    """The registered block ledger a record written to `out` belongs to, or None. A registered path is that ledger. A
    path outside the repository named as exactly one registered ledger is a staging copy of it: a batch of re-signs
    files its records there (MACRO_LEDGER_OUT and its kin) and lands them into the ledger together. Any other path,
    in the tree or out, is no ledger."""
    paths = [l["path"] for l in json.load(open(REGISTRY))["ledgers"]]
    rel = os.path.relpath(os.path.abspath(out), ROOT)
    if rel in paths:
        return rel
    if rel == os.pardir or rel.startswith(os.pardir + os.sep):
        named = [p for p in paths if os.path.basename(p) == os.path.basename(out)]
        if len(named) == 1:
            return named[0]
    return None


def main():
    if len(sys.argv) >= 2 and sys.argv[1] == "--verify":
        if len(sys.argv) not in (3, 4):
            sys.exit("usage: record-sky130-signoff.py --verify <ledger.json> [run_root]")
        sys.exit(run_verify(sys.argv[2], sys.argv[3] if len(sys.argv) == 4 else RUN_ROOT))
    if len(sys.argv) != 4:
        sys.exit("usage: record-sky130-signoff.py <run_dir> <block> <out.json>\n"
                 "       record-sky130-signoff.py --verify <ledger.json> [run_root]")
    run_dir, block, out = sys.argv[1], sys.argv[2], sys.argv[3]
    # Every block ledger is registered with its class before its first record, so no lock or script can
    # see one ledger and miss another (sky130_ledger_registry_lock).
    if registered_as(out) is None:
        sys.exit(f"{out} is not a registered block ledger: add it, with its class, to {os.path.relpath(REGISTRY, ROOT)}")
    sig = extract(run_dir, block)
    # A block hardened outside the `logos_<block>` convention — a Caravel user macro lives in its
    # project's design directory — records where its runs are, so the verifier re-derives it
    # rather than skipping it as a block with no run directory.
    if design_of(run_dir) != "logos_" + block:
        sig["design"] = design_of(run_dir)

    write_record(out, block, sig)

    verdict = "SIGNOFF CLEAN" if sig["signoff_clean"] else "NOT CLEAN"
    print(f"== {block}: {verdict} ==")
    print(f"  GDS {sig['gds_bytes']}B  {sig['std_cells']} cells  DRC {sig['drc_violations']}  "
          f"LVS {sig['lvs_errors']}(clean={sig['lvs_clean']})  XOR {sig['xor_differences']}  "
          f"ant {sig['antenna_violations']}/{sig['antenna_violators']}  route {sig['routing_violations']}")
    print(f"  setup {sig['setup_worst_slack_ns']}ns hold {sig['hold_worst_slack_ns']}ns  "
          f"violated {sig['setup_violated_paths']}/{sig['hold_violated_paths']}  "
          f"IRdrop {sig['ir_drop_worst_mv']}mV  multicorner {sig['multi_corner']}")
    sys.exit(0 if sig["signoff_clean"] else 1)


if __name__ == "__main__":
    main()
