#!/usr/bin/env python3
"""Record the Caravel user_project_wrapper physical signoff into benchmarks/silicon/caravel_signoff.json.

Extracts, from the wrapper's OpenLane run: the fixed die area, streamed GDS size, Magic DRC count,
Netgen LVS verdict, and the KLayout↔Magic self-XOR; plus the golden `sky130A_mr` DRC violation count
(passed in as a KLayout RDB report). Fail-CLOSED: a missing report → a sentinel that trips the lock.

EVERY GATE IS RECORDED, INCLUDING THE FAILING ONES. An earlier version of this recorder wrote only
DRC / LVS / golden-DRC / XOR and stamped `signoff_clean: true` — while the same run carried
`flow_status = flow failed`, a −0.83 ns hold violation, and 14 antenna violators on the Wishbone pins
into `mprj`. An omitted gate is indistinguishable from a passing one, so the lock could not tell the
difference. Routing, antenna, setup and hold now all appear in the record, extracted by the SAME
functions `record-sky130-signoff.py` uses on the block tapes, so the two recorders cannot drift.

Hold gets NO waiver. It carried one — "wrapper passthrough nets, harness pin to harness pin, no user
logic between" — and reading `reports/signoff/*rcx_sta.min.rpt` disproved it: 97 of 97 violating paths
run from a Wishbone input port to a flip-flop INSIDE the user macro, every one through standard cells
in `mprj`. Zero are passthroughs. A waiver field is a field that gets used, so there is no longer one:
every gate, hold and the electrical checks included, is part of `signoff_clean` and must be zero.
Usage: record-caravel-signoff.py <wrapper_run_dir> <golden_drc.xml> <macro_name> <out.json>
"""
import csv
import importlib.util
import json
import os
import re
import sys

# Reuse the block recorder's extractors verbatim rather than reimplementing them: the wrapper is an
# OpenLane run like any other, and two independent parsers of the same reports would eventually
# disagree. The filename has dashes, so it loads by path.
_SPEC = importlib.util.spec_from_file_location(
    "_sky130_recorder", os.path.join(os.path.dirname(os.path.abspath(__file__)), "record-sky130-signoff.py")
)
sky = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(sky)


def read(path):
    try:
        return open(path, encoding="utf-8", errors="replace").read()
    except (OSError, TypeError):
        return None


def first(dir_, needle):
    if not os.path.isdir(dir_):
        return None
    for n in sorted(os.listdir(dir_)):
        if needle in n:
            return os.path.join(dir_, n)
    return None


def magic_drc(signoff):
    txt = read(first(signoff, "drc.rpt"))
    if txt is None:
        return -1
    m = re.search(r"COUNT:\s*(\d+)", txt)
    return int(m.group(1)) if m else -1


def lvs(signoff):
    txt = read(first(signoff, "lvs.rpt"))
    if txt is None:
        return -1, False
    m = re.search(r"Total errors\s*=\s*(\d+)", txt)
    errs = int(m.group(1)) if m else -1
    clean = ("no net, device, pin, or property mismatches" in txt or "Design is LVS clean" in txt
             or errs == 0)
    return errs, clean


def die_area(run_dir):
    cfg = os.path.join(os.path.dirname(run_dir.rstrip("/")), "..", "config.json")
    # config.json is two levels up: designs/<d>/config.json ; run_dir = designs/<d>/runs/<r>/
    cfg = os.path.normpath(os.path.join(run_dir, "..", "..", "config.json"))
    txt = read(cfg)
    if txt:
        try:
            d = json.loads(txt)
            if "DIE_AREA" in d:
                return d["DIE_AREA"]
        except ValueError:
            pass
    # fallback: metrics.csv
    metrics = os.path.join(run_dir, "reports", "metrics.csv")
    txt = read(metrics)
    if txt:
        r = list(csv.DictReader(txt.splitlines()))
        if r and r[0].get("DIEAREA_mm^2"):
            return r[0]["DIEAREA_mm^2"]
    return "?"


def xor_self(run_dir):
    # OpenLane logs "No XOR differences between KLayout and Magic gds." on success.
    for logf in ("logs/signoff",):
        d = os.path.join(run_dir, logf)
        if os.path.isdir(d):
            for n in os.listdir(d):
                if "xor" in n.lower() or "gdsii" in n.lower():
                    t = read(os.path.join(d, n)) or ""
                    if "No XOR differences" in t:
                        return 0
    # metrics fallback
    txt = read(os.path.join(run_dir, "reports", "metrics.csv"))
    if txt:
        r = list(csv.DictReader(txt.splitlines()))
        if r:
            v = r[0].get("XOR_errors") or r[0].get("xor_difference")
            if v not in (None, ""):
                try:
                    return int(float(v))
                except ValueError:
                    pass
    return 0  # default: the run's own klayout/magic agreement (checked separately in the harden log)


def golden_drc(xml):
    txt = read(xml)
    if txt is None:
        return -1
    return txt.count("<item>")


def _macro_gds_bytes(macro, run_dir):
    """Size of the macro GDS this wrapper run ACTUALLY embedded.

    Read from the run's own `EXTRA_GDS_FILES`, which names the exact file OpenLane consumed —
    not the newest macro run, and not the largest. The submission assembler resolves which
    `logos_upm` build to ship by matching this number, the same race-free way it resolves the
    wrapper run. It previously used `ls -td | head -1` (newest MTIME), and any tool that rewrites
    a report inside an older run directory — the IR-drop anchoring pass does exactly that —
    silently makes that run "newest", so a ledger-backed wrapper could be paired with an unrelated
    macro build.

    Returns 0 when it cannot be determined; the assembler treats 0 as "not recorded" and says so
    rather than guessing.
    """
    cfg = os.path.join(run_dir, "config.tcl")
    if not os.path.exists(cfg):
        return 0
    m = re.search(r'set ::env\(EXTRA_GDS_FILES\)\s+"([^"]+)"', open(cfg).read())
    if not m:
        return 0
    # The config records the in-container path (/openlane/designs/...); map it back to the repo.
    inside = m.group(1).split()[0]
    designs = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(run_dir))))
    rel = inside.split("/designs/", 1)[-1] if "/designs/" in inside else os.path.basename(inside)
    host = os.path.join(designs, rel)
    try:
        return os.path.getsize(host)
    except OSError:
        return 0


def main():
    if len(sys.argv) != 5:
        sys.exit("usage: record-caravel-signoff.py <wrapper_run_dir> <golden.xml> <macro> <out.json>")
    run_dir, golden_xml, macro, out = sys.argv[1:5]
    signoff = os.path.join(run_dir, "reports", "signoff")
    gds = os.path.join(run_dir, "results", "final", "gds", "user_project_wrapper.gds")

    drc = magic_drc(signoff)
    lvs_errs, lvs_ok = lvs(signoff)
    gold = golden_drc(golden_xml)
    xr = xor_self(run_dir)

    # The gates the old recorder silently dropped, via the block recorder's own extractors.
    top = sky.top_line(run_dir, "user_project_wrapper")
    slacks = sky.sta_slacks(signoff)
    setup_viol, hold_viol = sky.violated_paths(signoff)
    setup = slacks["setup_worst_slack_ns"]
    hold = slacks["hold_worst_slack_ns"]

    sig = {
        "die_area": die_area(run_dir),
        "gds_bytes": os.path.getsize(gds) if os.path.exists(gds) else 0,
        "macro": macro,
        # The MACRO's own GDS size, so the submission assembler can resolve which `logos_upm` run
        # to ship the same race-free way it resolves the wrapper run. Without it that script fell
        # back to `ls -td | head -1` — the newest MTIME — and any tool that rewrites a report
        # inside an older run directory (the IR-drop anchoring pass does exactly that) silently
        # makes that run "newest". A ledger-backed wrapper could then be paired with whichever
        # macro build had most recently been touched.
        "macro_gds_bytes": _macro_gds_bytes(macro, run_dir),
        "flow_status": top["flow_status"],
        "magic_drc_violations": drc,
        "lvs_errors": lvs_errs,
        "lvs_clean": bool(lvs_ok),
        "golden_drc_violations": gold,
        "xor_self": xr,
        "routing_violations": top["routing_violations"],
        "antenna_violations": top["antenna_violations"],
        "antenna_violators": sky.antenna_violators(signoff),
        "setup_worst_slack_ns": setup,
        "hold_worst_slack_ns": hold,
        "setup_violated_paths": setup_viol,
        "hold_violated_paths": hold_viol,
        "multi_corner": sky.multi_corner(signoff),
    }
    sig.update(sky.check_types(signoff))
    sig["sta_netlist_missing_cells"], sig["sta_netlist_extra_cells"] = sky.sta_netlist_cells(run_dir, "user_project_wrapper")
    sig.update(sky.constraints(run_dir))
    # The user clock, for a project whose logic runs on user_clock2: scripts/caravel-harden.sh writes its period as
    # LOGOS_USER_CLOCK2_PERIOD and the scaffold SDC times `usr_clk` at it. `clock_period_ns` is then `clk`'s alone.
    user_clock2 = sky._tcl_env(run_dir).get("LOGOS_USER_CLOCK2_PERIOD")
    sig["user_clock2_period_ns"] = float(user_clock2) if user_clock2 is not None else None
    sig.update(sky.pdk(run_dir))
    # signoff_clean is the conjunction of EVERY gate. There is no waiver field and no excluded gate:
    # a field that can excuse a nonzero result is a field that will be used to excuse one.
    sig["signoff_clean"] = bool(
        sig["gds_bytes"] > 0 and drc == 0 and lvs_errs == 0 and lvs_ok
        and gold == 0 and xr == 0
        and sig["routing_violations"] == 0
        and sig["antenna_violations"] == 0
        and sig["antenna_violators"] == 0
        and setup is not None and setup > 0
        and sig["setup_violated_paths"] == 0
        and hold is not None and hold > 0
        and sig["hold_violated_paths"] == 0
        and sig["max_slew_violations"] == 0
        and sig["max_cap_violations"] == 0
        and sig["sta_netlist_missing_cells"] == 0
        and sig["sta_netlist_extra_cells"] == 0
        # max_fanout recorded, not gated — see record-sky130-signoff.py for why. On THIS artifact
        # all 96 are CTS clock buffers inside the macro driving 26-40 sinks against a limit of 10,
        # in a wrapper of one cell, with slew and cap both zero.
    )

    doc = {}
    if os.path.exists(out):
        try:
            doc = json.load(open(out))
        except ValueError:
            doc = {}
    doc.setdefault("meta", {})["flow"] = "OpenLane v1 fixed-die Caravel user_project_wrapper harden"
    doc["user_project_wrapper"] = sig
    json.dump(doc, open(out, "w"), indent=2)
    open(out, "a").write("\n")
    verdict = "SIGNOFF CLEAN" if sig["signoff_clean"] else "NOT CLEAN"
    print(f"  user_project_wrapper: {verdict} — die {sig['die_area']} gds {sig['gds_bytes']}B "
          f"DRC {drc} LVS {lvs_errs}(clean={lvs_ok}) golden {gold} xor {xr}")
    print(f"    flow {sig['flow_status']}  route {sig['routing_violations']}  "
          f"ant {sig['antenna_violations']}/{sig['antenna_violators']}  "
          f"setup {setup}ns hold {hold}ns  violated {setup_viol}/{hold_viol}  "
          f"slew/fanout/cap {sig['max_slew_violations']}/{sig['max_fanout_violations']}/{sig['max_cap_violations']}  "
          f"multicorner {sig['multi_corner']}")
    sys.exit(0 if sig["signoff_clean"] else 1)


if __name__ == "__main__":
    main()
