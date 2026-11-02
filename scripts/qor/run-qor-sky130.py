#!/usr/bin/env python3
"""The Sky130 runs of the comparative-synthesis bench: each candidate inside its fold wrapper, out of context, through
OpenLane (scripts/openlane-run.sh) at a clock the bench sets, recorded by scripts/record-sky130-signoff.py into
benchmarks/silicon/qor_sky130.json — a `comparative` ledger of benchmarks/silicon/sky130_ledgers.json — with what was
staged for each run recorded in benchmarks/silicon/qor_sky130_stage.json.

    run-qor-sky130.py stage <kernel> <candidate> --period P       stage one attempt; no flow run
    run-qor-sky130.py run <kernel> <candidate> --period P         stage, run OpenLane, record
    run-qor-sky130.py search <kernel> <candidate> [--start P]     the candidate's own minimum period: a run at P, then at
                                                                  the period its runs show it needs, until none is shorter
    run-qor-sky130.py iso <kernel>                                every candidate of the kernel at its iso period: the
                                                                  longest own minimum among them
    run-qor-sky130.py control <kernel> <candidate> --period P|--at-iso
                                                                  the attempt on OpenLane as released, at the commit
                                                                  the overlay is written over (scripts/openlane-run.sh
                                                                  OL_STOCK=1): the overlay's contribution is the
                                                                  difference from the attempt's run
    run-qor-sky130.py power <kernel> <candidate> [--period P]     the candidate's functional harness hardened as a block
                                                                  of its own at the kernel's iso period (or P), whose
                                                                  gate-level simulation scripts/qor/record-qor-power.py
                                                                  measures; after a miss, once more at the period the
                                                                  run shows it needs
    run-qor-sky130.py plan                                        every candidate's attempts, own minimum and iso period
    run-qor-sky130.py verify                                      every stage row against its directory and its sources
    run-qor-sky130.py room [--within S]                           wait until the box has room for a flow (as each flow
                                                                  waits before it starts), or give up after S seconds

An attempt is a block of its own, `qorsyn_<candidate>_p<period>` (the period's point written `_`): the candidate in a
fold wrapper of that name, staged from benchmarks/silicon/qor/sky130/<block>/ — the directory scripts/openlane-run.sh
copies from (generated, not tracked; its files' SHA3-256 are in the stage row). SystemVerilog is staged under a `.v`
name: OpenLane reads every source with `read_verilog -sv`. It flows in the bench's own tree, never in tools/OpenLane,
where the signoff blocks and every other session flow: tools/qor/openlane-bench, OpenLane as released at the overlay's
base with the overlay laid over it (scripts/openlane-run.sh OL_BENCH=1). A run the global router refuses for congestion
is repeated with core utilization and placement density lowered together (FLOORPLANS, one ladder for every candidate),
as is one that meets timing and fails signoff only on antenna pins every repair left; the stage row records the
floorplan its run used and each floorplan left before it, with why. A candidate closes at a period when its run is signoff-clean:
every corner meets setup and hold, and DRC, LVS, antenna and routing are clean. The period after a miss is
`signoff_blocks::honest_period_ns` (examples/honest_period): the miss plus 5%, on the 0.5 ns grid — the rule the
signoff blocks are held to. Every stage row records the flow it ran: the OpenLane commit and the SHA3-256 of the
overlay laid over it, or none for a control, whose block is `qorsyn_<candidate>_p<period>_stock`, which flows in
tools/qor/openlane-stock and whose run is recorded in benchmarks/silicon/qor_sky130_control.json; controls never count
toward an own minimum.

A power block, `qorpwr_<candidate>_p<period>`, is the candidate inside the harness of its functional gate
(codegen_sva::qor_harness, emitted under the block's name): the stimulus generator, the candidate kept whole as
`u_dut`, the signature register — a block that drives itself from its clock and reset, so its gate-level simulation
is the candidate at work on the gate's stimulus. Its run is recorded in benchmarks/silicon/qor_sky130_power.json and
never counts toward an own minimum either. Every stage row names its role: attempt, control or power.

OpenLane takes most of the box's memory at its peak, so a flow starts only when the box has room for it: no other
OpenLane flow runs but those the bench was told it may run beside (BENCH_COEXIST), and FLOW_FREE_GB of memory is free.
"""
import argparse
import concurrent.futures
import fnmatch
import importlib.util
import json
import os
import re
import shutil
import signal
import subprocess
import sys
import threading
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import qorlib  # noqa: E402

LEDGER = os.path.join(qorlib.SIL, "qor_sky130.json")
CONTROL_LEDGER = os.path.join(qorlib.SIL, "qor_sky130_control.json")
POWER_LEDGER = os.path.join(qorlib.SIL, "qor_sky130_power.json")
# Each role: the block name of a candidate at a period, and the ledger its runs are recorded in.
ROLES = {
    "attempt": ("qorsyn_%s_p%s", LEDGER),
    "control": ("qorsyn_%s_p%s_stock", CONTROL_LEDGER),
    "power": ("qorpwr_%s_p%s", POWER_LEDGER),
}
STAGE_ROOT = os.path.join(qorlib.SIL, "qor", "sky130")
# The floorplans a run is tried in (qorlib.SKY130_FLOORPLANS); each stage row records the floorplan its run used and
# each left before it, with why: congestion, or antenna pins no repair could fit with timing met.
FLOORPLANS = qorlib.SKY130_FLOORPLANS
# The global router's verdicts that a floorplan is too tight for its wiring: GRT-0119 after the first global route,
# GRT-0232 when the overflow outlasts the incremental re-route the repairs run.
CONGESTION = ("GRT-0119", "GRT-0232")
# How long a flow waits for room on the box, how often it looks, and the memory a bench flow needs free to start: its
# own peak with room to spare beside a flow it may run beside (openlane-run.sh's default floor is for the box's largest
# blocks, run alone).
FLOW_WAIT_S = 6 * 3600
FLOW_POLL_S = 60
FLOW_FREE_GB = 40
# A flow starts only with this much free on the filesystem the bench's trees are on: a run that starts into a full disk
# dies mid-route and takes the flows beside it down with it.
FLOW_FREE_DISK_GB = 50
BENCH_TREES = os.path.join(qorlib.ROOT, "tools", "qor")
# The designs the bench stages: its attempts and controls (qorsyn) and its power blocks (qorpwr). BENCH_FLOWS of them
# may run at once (one when unset).
BENCH_DESIGNS = ("logos_qorsyn_*", "logos_qorpwr_*")
# How long a flow's start holds the next one back: until its container is up, so the next start counts it.
START_HOLD_S = 120
STARTING = threading.Lock()
# How often a running flow's container memory is read.
FLOW_SAMPLE_S = 5
HONEST = os.path.join(qorlib.TARGET, "debug", "examples", "honest_period")
HEADER = {
    "_generated_by": "scripts/qor/run-qor-sky130.py (never hand-edited; verify re-hashes every staged file)",
    "what": "What each Sky130 attempt of the comparative-synthesis bench staged: the candidate's files and the fold wrapper "
            "they were run in, each with its SHA3-256 and origin. The attempt's results are its block's row of "
            "benchmarks/silicon/qor_sky130.json, recorded and re-derived by scripts/record-sky130-signoff.py.",
}
MAX_ATTEMPTS = 4


def block_of(candidate, period, role="attempt"):
    return ROLES[role][0] % (candidate, ("%g" % period).replace(".", "_"))


def flow_identity(role):
    """The flow a run of `role` goes through (qorlib.flow_identity): a control runs OpenLane as released."""
    return qorlib.flow_identity(role == "control")


def current_row(block, role="attempt"):
    """The block's recorded run when it went through the flow a run of `role` goes through now; None when it has not
    run, or ran under another flow — a superseded attempt, which is run again rather than read."""
    staged = next((s for s in qorlib.load("sky130_stage", {"rows": []})["rows"] if s["block"] == block), None)
    if staged is None or staged.get("flow") != flow_identity(role):
        return None
    return ledger_row(block, role)


def top_clock(path, block):
    """The clock port of the staged top `block` (its source at `path`). Every top the bench stages — a fold wrapper, a
    power block's harness — clocks the block through `input clk`, whatever the candidate inside names its own clock;
    a top without it is refused before a flow runs."""
    m = re.search(r"module\s+%s\s*\(([^)]*)\)" % re.escape(block), open(path).read())
    ports = [p.strip() for p in m.group(1).split(",")] if m else []
    if "input clk" not in ports:
        sys.exit("%s: its top declares no `input clk` (ports: %s)" % (block, ", ".join(ports) or "none"))
    return "clk"


def materialise(run_dir, kernel, candidate):
    """The candidate's record with its RTL and its fold wrapper in `run_dir`."""
    if kernel == "K7":
        return qorlib.emit_fold(run_dir, candidate)
    rec = qorlib.emit(run_dir, kernel, candidate, "shaped", 16)
    rec.update(qorlib.wrap_candidate(run_dir))
    return rec


READMEM = re.compile(r"\$readmem[bh]\s*\(\s*([A-Za-z_]\w*)")


def init_stand_ins(dest):
    """The init files a staged design's memory templates default to and no tool emitted, as (name, why): a parameter a
    `$readmem` reads, whose default names a file the staging directory lacks (Bambu's `array_a.mem`; every instance
    overrides it with its own `array_ref_*.mem`). OpenLane's yosys reads every template at parse time and refuses the
    design for the missing default; the FPGA flows read with -defer and never parse it. Its stand-in is empty: no
    instance reads it."""
    names, out = set(os.listdir(dest)), []
    for f in sorted(n for n in names if n.endswith(".v")):
        text = open(os.path.join(dest, f), errors="replace").read()
        for param in sorted(set(READMEM.findall(text))):
            for default in sorted(set(re.findall(r"\b%s\s*=\s*\"([^\"]+)\"" % re.escape(param), text))):
                if default not in names and default not in [n for n, _ in out]:
                    out.append((default, "%s: the default of %s, which no tool emitted" % (f, param)))
    return out


def stage(kernel, candidate, period, role="attempt"):
    """Fill the run's staging directory and record its stage row; return (block, stage row). A power block is the
    candidate inside its functional harness; a K7 controller's, which has none, is the controller with every port
    passed through, driven at gate level by the K7 traffic testbench."""
    block = block_of(candidate, period, role)
    with qorlib.scratch() as run_dir:
        if role == "power" and kernel != "K7":
            rec = qorlib.emit(run_dir, kernel, candidate, "shaped", 16, module=block)
        else:
            rec = materialise(run_dir, kernel, candidate)
        name = rec["candidate"]["name"]
        if name != candidate:
            sys.exit("%s: the emitter names it %s" % (candidate, name))
        wrapper = os.path.join(run_dir, block + ".v")
        record = os.path.join(run_dir, "candidate.json")
        if kernel == "K7":
            d = qorlib.fold_designs()[candidate]
            kind, name_arg = ("wrap-pass", block) if role == "power" else ("wrap", block[len("qorsyn_"):])
            cmd = [qorlib.EMIT_FOLD, kind, name_arg, rec["candidate"]["top"], d["clock"], d["reset"], d["polarity"],
                   os.path.join(run_dir, "ports.json"), wrapper]
        elif role != "power":
            cmd = [qorlib.EMIT_FOLD, "wrap-candidate", record, wrapper, block[len("qorsyn_"):]]
        else:
            cmd = None
        if cmd:
            r = subprocess.run(cmd, capture_output=True, text=True)
            if r.returncode != 0:
                sys.exit("emit_qor_fold: %s" % r.stderr.strip())
        dest = os.path.join(STAGE_ROOT, block)
        if os.path.isdir(dest):
            shutil.rmtree(dest)
        os.makedirs(dest)
        files = []
        for f in rec["files"]:
            staged = f[:-3] + ".v" if f.endswith(".sv") else f
            shutil.copyfile(os.path.join(run_dir, f), os.path.join(dest, staged))
            files.append({"name": staged, "from": f, "sha3_256": qorlib.sha3_file(os.path.join(dest, staged))})
        for stand_in, why in init_stand_ins(dest):
            open(os.path.join(dest, stand_in), "w").close()
            files.append({"name": stand_in, "from": None, "stand_in": why,
                          "sha3_256": qorlib.sha3_file(os.path.join(dest, stand_in))})
        shutil.copyfile(wrapper, os.path.join(dest, block + ".v"))
        made = {("power", False): "emit_qor_harness --as (the functional harness)",
                ("power", True): "emit_qor_fold wrap-pass (the controller's ports passed through)"}
        files.append({"name": block + ".v", "from": made.get((role, kernel == "K7"), "emit_qor_fold (the fold wrapper)"),
                      "sha3_256": qorlib.sha3_file(os.path.join(dest, block + ".v"))})
        clock = top_clock(os.path.join(dest, block + ".v"), block)
        row = {"key": block, "block": block, "kernel": kernel, "candidate": name, "candidate_top": rec["candidate"]["top"],
               "source_kind": rec["source_kind"], "period_ns": period, "clock_port": clock, "files": files,
               "staged_at": qorlib.now(), "role": role, "flow": flow_identity(role),
               "reproduce": "python3 scripts/qor/run-qor-sky130.py %s %s %s --period %g"
                            % ({"attempt": "run"}.get(role, role), kernel, candidate, period)}
        if kernel == "K7":
            row.update({"rtl_sha3_256": rec["rtl_sha3_256"], "generate": rec["generate"]})
    qorlib.update("sky130_stage", HEADER, row)
    return block, row


def ledger_row(block, role="attempt"):
    """The block's recorded run: None when it has none, when its staging has run no flow yet (no `flow_rc`), or when
    that flow recorded nothing — a record an older flow of the block left in the ledger is not its run."""
    staged = next((s for s in qorlib.load("sky130_stage", {"rows": []})["rows"] if s["block"] == block), None)
    ledger = ROLES[role][1]
    if (staged is not None and ("flow_rc" not in staged or staged.get("refused"))) or not os.path.exists(ledger):
        return None
    return (json.load(open(ledger)).get("blocks") or {}).get(block)


def run_root(role):
    """The OpenLane designs/ directory a run of `role` is in: its ledger's, by the ledger registry."""
    return qorlib.run_root(ROLES[role][1])


def runs_of(block, role):
    d = os.path.join(run_root(role), "logos_" + block, "runs")
    return set(os.listdir(d)) if os.path.isdir(d) else set()


def congested(block, role, run):
    """The congestion verdict the global router refused the OpenLane run `run` of `block` with, or None."""
    logs = os.path.join(run_root(role), "logos_" + block, "runs", run, "logs", "routing")
    if not os.path.isdir(logs):
        return None
    for f in sorted(os.listdir(logs)):
        if f.endswith(".log"):
            text = open(os.path.join(logs, f), errors="replace").read()
            for code in CONGESTION:
                if code in text:
                    return code
    return None


LINT_ERROR = re.compile(r"^%Error: (?:\S*/)?([^/\s:]+:\d+:\d+: .+)$", re.M)


def lint_refusal(block, role, run):
    """Why OpenLane's lint step (Verilator, step 0) refused the OpenLane run `run` of `block`: its first error, the
    file named without its directory; None when the linter raised none."""
    log = os.path.join(run_root(role), "logos_" + block, "runs", run, "logs", "synthesis", "linter.log")
    m = LINT_ERROR.search(open(log, errors="replace").read()) if os.path.exists(log) else None
    return "OpenLane's lint step (Verilator) refused the RTL: %s" % m.group(1).strip() if m else None


def _recorder():
    spec = importlib.util.spec_from_file_location("record_sky130_signoff",
                                                  os.path.join(qorlib.ROOT, "scripts", "record-sky130-signoff.py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


RECORDER = _recorder()


def recorded(block, role, new):
    """Whether the flow that created the runs `new` of `block` recorded: the block's record in its ledger is what the
    recorder extracts from the newest of them. Flows run side by side and rewrite the ledger as each finishes, so a
    ledger written while this flow ran is no evidence that it recorded; its own run's record is. A run the recorder
    cannot read is one it did not record."""
    ledger = ROLES[role][1]
    if not new or not os.path.exists(ledger):
        return False
    rec = (json.load(open(ledger)).get("blocks") or {}).get(block)
    if rec is None:
        return False
    try:
        return rec == RECORDER.extract(os.path.join(run_root(role), "logos_" + block, "runs", sorted(new)[-1]), block)
    except Exception:
        return False


def design_of(cid):
    """The design the container's OpenLane flow runs (`-design <name>` in its command), or `unknown`."""
    args = subprocess.run(["docker", "inspect", "-f", "{{json .Args}}", cid], capture_output=True, text=True).stdout
    m = re.search(r"-design (\S+?)[\\\" ]", args)
    return m.group(1) if m else "unknown"


def box_flows():
    """The flows that take the box's memory, running in any tree and any session — OpenLane's and mpw_precheck's —
    each as (`<image> <name>`, the design it runs, or `unknown`)."""
    out = subprocess.run(["docker", "ps", "--format", "{{.ID}} {{.Image}} {{.Names}}"], capture_output=True,
                         text=True).stdout
    flows = []
    for line in out.splitlines():
        cid, _, desc = line.partition(" ")
        if not desc.split() or not any(k in desc.split()[0] for k in ("openlane", "precheck")):
            continue
        flows.append((desc, design_of(cid)))
    return flows


def flow_container(design):
    """The full ID of the running container whose flow runs exactly `design`, or None."""
    for cid in subprocess.run(["docker", "ps", "-q", "--no-trunc"], capture_output=True, text=True).stdout.split():
        if design_of(cid) == design:
            return cid
    return None


def container_peak_gb(cid):
    """The most memory the container has held, page cache included (its cgroup's memory.peak), in GiB; None when it
    cannot be read."""
    try:
        return int(open("/sys/fs/cgroup/system.slice/docker-%s.scope/memory.peak" % cid).read()) / 2 ** 30
    except (OSError, ValueError):
        return None


def mem_available_gb():
    for line in open("/proc/meminfo"):
        if line.startswith("MemAvailable:"):
            return int(line.split()[1]) / (1024 * 1024)
    return 0.0


def disk_free_gb():
    """Gigabytes free on the filesystem the bench's OpenLane trees are on."""
    return shutil.disk_usage(BENCH_TREES).free / 1e9


def trim(block, role, runs):
    """Drop the database and intermediates of runs a flow did not record (`results/`, `tmp/`, the bulk of a run),
    keeping their logs and reports: the evidence of why they did not."""
    for run in runs:
        for part in ("results", "tmp"):
            shutil.rmtree(os.path.join(run_root(role), "logos_" + block, "runs", run, part), ignore_errors=True)


def room_for_a_flow():
    """Wait until the box has room for a bench flow: no other session's OpenLane or mpw_precheck run but those whose
    design the bench was told it may run beside (BENCH_COEXIST, space-separated globs over the design), fewer than
    BENCH_FLOWS of the bench's own (one when unset), FLOW_FREE_GB of memory free and FLOW_FREE_DISK_GB of disk. A
    search keeps its next attempt when another session's flow comes and goes between two of its own. A wait past
    FLOW_WAIT_S refuses the flow."""
    beside = os.environ.get("BENCH_COEXIST", "").split()
    limit = int(os.environ.get("BENCH_FLOWS") or 1)
    waited = 0
    while True:
        flows = box_flows()
        own = [d for _, d in flows if any(fnmatch.fnmatchcase(d, g) for g in BENCH_DESIGNS)]
        held = ["%s, design %s" % (desc, design) for desc, design in flows
                if not any(fnmatch.fnmatchcase(design, g) for g in beside + list(BENCH_DESIGNS))]
        if len(own) >= limit:
            held.append("%d of the bench's own flows, as many as BENCH_FLOWS lets run (%s)" % (len(own), ", ".join(own)))
        free = mem_available_gb()
        disk = disk_free_gb()
        if not held and free >= FLOW_FREE_GB and disk >= FLOW_FREE_DISK_GB:
            return
        if waited >= FLOW_WAIT_S:
            if held:
                sys.exit("another OpenLane flow is still running after %d s (%s): no room for a flow"
                         % (waited, "; ".join(held)))
            if disk < FLOW_FREE_DISK_GB:
                sys.exit("only %d GB free on the disk after %d s, of the %d GB a bench flow needs: no room for a flow"
                         % (disk, waited, FLOW_FREE_DISK_GB))
            sys.exit("only %d GB free after %d s, of the %d GB a bench flow needs: no room for a flow"
                     % (free, waited, FLOW_FREE_GB))
        if waited == 0:
            print("waiting for room: %s" % ("; ".join(held) if held else "%d GB of memory and %d GB of disk free, of the "
                                              "%d GB and %d GB a bench flow needs" % (free, disk, FLOW_FREE_GB,
                                                                                      FLOW_FREE_DISK_GB)), flush=True)
        time.sleep(FLOW_POLL_S)
        waited += FLOW_POLL_S


def flow(block, row, deadline):
    """scripts/openlane-run.sh on the staged block in the floorplan its row names, in a tree of the bench's own — an
    attempt and a power block in tools/qor/openlane-bench (OL_BENCH=1), the overlay laid over it; a control in
    tools/qor/openlane-stock (OL_STOCK=1) — its process group and its OpenLane container killed past `deadline`
    seconds. The most memory its container held while it ran goes on `row` as flow_peak_mem_gb. Returns (returncode,
    timed out)."""
    role = row["role"]
    rtl = [os.path.join("qor", "sky130", block, f["name"]) for f in row["files"]]
    control = role == "control"
    floorplan = row.get("floorplan") or FLOORPLANS[0]
    env = dict(os.environ, OUT_JSON=ROLES[role][1], OL_STOCK="1" if control else "0", OL_BENCH="0" if control else "1",
               COMPARATIVE="1", FP_CORE_UTIL="%d" % floorplan["fp_core_util"], OL_MIN_FREE_GB="%d" % FLOW_FREE_GB,
               ROUTING_CORES=qorlib.routing_cores())
    env.pop("PL_TARGET_DENSITY", None)
    if floorplan["pl_target_density"] is not None:
        env["PL_TARGET_DENSITY"] = "%g" % floorplan["pl_target_density"]
    design, cid, peak = "logos_" + block, None, None
    # A start holds the next flow's back until this flow's container is up (or START_HOLD_S has passed, or the flow has
    # ended), so the next one's room check counts it.
    STARTING.acquire()
    starting = True
    try:
        room_for_a_flow()
        p = subprocess.Popen(["bash", os.path.join(qorlib.ROOT, "scripts", "openlane-run.sh"), block,
                              row["clock_port"], "%g" % row["period_ns"]] + rtl, cwd=qorlib.ROOT, env=env,
                             start_new_session=True)
        t0 = time.monotonic()
        while True:
            try:
                rc, late = p.wait(timeout=FLOW_SAMPLE_S), False
                break
            except subprocess.TimeoutExpired:
                pass
            cid = cid or flow_container(design)
            got = container_peak_gb(cid) if cid else None
            if got is not None:
                peak = max(peak or 0.0, got)
            now = time.monotonic()
            if starting and (cid or now - t0 >= START_HOLD_S):
                STARTING.release()
                starting = False
            if now - t0 >= deadline:
                os.killpg(p.pid, signal.SIGTERM)
                try:
                    p.wait(timeout=60)
                except subprocess.TimeoutExpired:
                    os.killpg(p.pid, signal.SIGKILL)
                    p.wait()
                cid = cid or flow_container(design)
                if cid:
                    subprocess.run(["docker", "kill", cid], capture_output=True)
                rc, late = p.returncode, True
                break
    finally:
        if starting:
            STARTING.release()
    row["flow_peak_mem_gb"] = round(peak, 2) if peak is not None else None
    return rc, late


def antenna_left(rec):
    """Did the run meet setup and hold and every other signoff check, and fail only on antenna pins every repair
    left? A looser floorplan gives the repair room; no period does."""
    others = all(rec.get(k) == 0 for k in ("drc_violations", "routing_violations", "xor_differences",
                                           "max_slew_violations", "max_cap_violations", "max_fanout_violations"))
    return (rec.get("signoff_clean") is False and others and rec.get("lvs_clean") is True
            and all(rec.get(k) is not None and rec[k] >= 0 for k in ("setup_worst_slack_ns", "hold_worst_slack_ns"))
            and (rec.get("antenna_violators") or 0) > 0)


def run(kernel, candidate, period, deadline, role="attempt"):
    """Stage the block and run its flow in each of FLOORPLANS in turn: a run the global router refuses for congestion,
    or one that meets timing and fails signoff only on antenna pins every repair left, is repeated in the next, and
    the stage row names each floorplan left and why. A run is recorded when the ledger holds its own run's record
    (`recorded`), whatever other flows wrote meanwhile."""
    block, staged = stage(kernel, candidate, period, role)
    ledger = ROLES[role][1]
    loosened = []
    for floorplan in FLOORPLANS:
        last_one = floorplan is FLOORPLANS[-1]
        row = dict(staged, floorplan=floorplan, loosened=list(loosened))
        before = runs_of(block, role)
        t0 = time.monotonic()
        rc, timed_out = flow(block, row, deadline)
        row.update(flow_rc=rc, flow_wall_s=round(time.monotonic() - t0, 1), deadline_s=deadline, refused=None)
        new = sorted(runs_of(block, role) - before)
        if timed_out:
            row["refused"] = "OpenLane did not finish within %d s" % deadline
            trim(block, role, new)
        elif not recorded(block, role, new):
            row["refused"] = "OpenLane exited %s without a recorded run" % rc
            congestion = congested(block, role, new[-1]) if new else None
            lint = lint_refusal(block, role, new[-1]) if new else None
            if lint:
                row["refused"] = lint
            trim(block, role, new)
            if congestion:
                row["refused"] = "global routing congestion (%s) %s %d%% core utilization" % (
                    congestion, "in every floorplan, down to" if last_one else "at", floorplan["fp_core_util"])
                if not last_one:
                    loosened.append(dict(floorplan, why="congestion"))
                    qorlib.update("sky130_stage", HEADER, row)
                    continue
        elif antenna_left((json.load(open(ledger)).get("blocks") or {}).get(block) or {}):
            loosened.append(dict(floorplan, why="antenna"))
            qorlib.update("sky130_stage", HEADER, row)
            continue
        break
    qorlib.update("sky130_stage", HEADER, row)
    rec = ledger_row(block, role)
    print("%-48s %s" % (block, row["refused"] or "clean %s, setup %s ns, hold %s ns, %s um2" % (
        rec["signoff_clean"], rec["setup_worst_slack_ns"], rec["hold_worst_slack_ns"], rec.get("std_cell_area_um2"))),
        flush=True)
    return block


def closes(rec):
    return bool(rec) and rec["signoff_clean"] is True


def honest(period, slack):
    if not os.path.exists(HONEST):
        sys.exit("build it first: cargo build -p logicaffeine-compile --example honest_period")
    return float(subprocess.run([HONEST, "%g" % period, "%g" % slack], capture_output=True, text=True, check=True).stdout)


def search(kernel, candidate, start, deadline):
    """The candidate's own minimum period. A run at `start`; after a miss, a run at the period its slowest corner
    needs; after a close with slack to spare, a run at the period its measured path needs — both by `honest_period`
    (the miss, or the measured path, plus 5%, on the 0.5 ns grid) — until a period comes round again, a run fails for
    a reason another period cannot fix (hold, DRC, LVS, antenna, routing), the flow records nothing, or MAX_ATTEMPTS
    have run. The own minimum is then the shortest period at which a run closed."""
    period, seen = start, set()
    for _ in range(MAX_ATTEMPTS):
        seen.add(period)
        rec = current_row(block_of(candidate, period)) or ledger_row(run(kernel, candidate, period, deadline))
        if rec is None:
            return
        slack = rec["setup_worst_slack_ns"]
        if closes(rec):
            nxt = honest(period - slack, -0.0)
        elif slack < 0:
            nxt = honest(period, slack)
        else:
            return
        if nxt in seen or (closes(rec) and nxt >= period):
            return
        period = nxt


def attempts(role="attempt"):
    """Every recorded run of `role` through the flow such a run goes through now, by candidate: [(period, block, ledger
    row)]. A superseded attempt counts toward nothing."""
    stage_rows = qorlib.load("sky130_stage", {"rows": []})["rows"]
    flow = flow_identity(role)
    out = {}
    for s in stage_rows:
        if s["role"] != role or s.get("flow") != flow:
            continue
        out.setdefault((s["kernel"], s["candidate"]), []).append(
            (s["period_ns"], s["block"], ledger_row(s["block"], role)))
    return {k: sorted(v) for k, v in out.items()}


def own_minimum(tries):
    closed = [p for p, _, rec in tries if closes(rec)]
    return min(closed) if closed else None


def plan():
    by_kernel = {}
    for (kernel, cand), tries in sorted(attempts().items()):
        by_kernel.setdefault(kernel, []).append((cand, tries, own_minimum(tries)))
    for kernel, cands in sorted(by_kernel.items()):
        mins = [m for _, _, m in cands if m is not None]
        print("%s  iso period %s" % (kernel, "%g ns" % max(mins) if mins else "not yet"))
        for cand, tries, m in cands:
            print("  %-36s own minimum %-8s attempts %s" % (cand, "%g" % m if m else "-", ", ".join(
                "%g%s" % (p, "*" if closes(rec) else "" if rec else "?") for p, _, rec in tries)))
            for role, what in (("control", "control on stock OpenLane"), ("power", "power block")):
                for p, _, rec in attempts(role).get((kernel, cand), []):
                    print("  %-36s %s at %g: %s" % ("", what, p, "closes" if closes(rec) else
                                                    "records no run" if rec is None else "misses"))


def iso_period(kernel):
    """The kernel's iso period: the longest own minimum among its candidates."""
    mins = [m for (k, _), tries in attempts().items() if k == kernel for m in [own_minimum(tries)] if m is not None]
    if not mins:
        sys.exit("%s: no candidate has an own minimum yet — search first" % kernel)
    return max(mins)


def traffic_driven():
    """The K7 controllers the traffic gate's testbenches drive (emit_qor_dram --list): the ones a power block can be
    measured for."""
    emit = os.path.join(qorlib.TARGET, "debug", "examples", "emit_qor_dram")
    if not os.path.exists(emit):
        sys.exit("build the emitter first: cargo build -p logicaffeine-compile --example emit_qor_dram")
    return subprocess.run([emit, "--list"], capture_output=True, text=True, check=True).stdout.split()


def power(kernel, candidate, period, deadline):
    """The candidate's power block at `period` (the kernel's iso period when none is given); after a run that misses
    setup, once more at the period its slowest corner needs. A power block measures energy at a clock it closes at."""
    if kernel == "K7" and candidate not in traffic_driven():
        sys.exit("K7 %s: no traffic testbench drives it (the K7 traffic gate drives %s)"
                 % (candidate, ", ".join(traffic_driven())))
    period = period if period is not None else iso_period(kernel)
    for _ in range(2):
        rec = current_row(block_of(candidate, period, "power"), "power") or \
            ledger_row(run(kernel, candidate, period, deadline, "power"), "power")
        if rec is None or closes(rec) or rec["setup_worst_slack_ns"] >= 0:
            return
        period = honest(period, rec["setup_worst_slack_ns"])


def together(task, items, jobs):
    """`task` on every item, `jobs` at a time; an item's refusal (its SystemExit) is printed and stops no other."""
    def one(item):
        try:
            task(item)
        except SystemExit as e:
            print("%s: %s" % (item, e), flush=True)
    with concurrent.futures.ThreadPoolExecutor(max_workers=max(1, jobs)) as pool:
        list(pool.map(one, items))


def gated_candidates(kernel):
    """The kernel's candidates that passed their functional gate on both stimulus modes; K7's, the designs it has."""
    if kernel == "K7":
        return sorted(json.load(open(os.path.join(qorlib.SIL, "qor", "kernels", "K7", "kernel.json")))["designs"])
    modes = {}
    for r in qorlib.load("function", {"rows": []})["rows"]:
        if r["kernel"] == kernel and r["pass"]:
            modes.setdefault(r["candidate"], set()).add(r["mode"])
    return sorted(c for c, m in modes.items() if m == {"shaped", "edge"})


def iso(kernel, deadline, jobs=1):
    """Every candidate not yet run at the kernel's iso period, at it, `jobs` at a time."""
    period = iso_period(kernel)
    together(lambda c: run(kernel, c, period, deadline),
             [c for (k, c), tries in sorted(attempts().items()) if k == kernel and not any(p == period for p, _, _ in tries)],
             jobs)


def kernel(kernel_, jobs, start, deadline, controls=(), starts=None):
    """A kernel whole: every candidate that passed its gate searched for its own minimum, `jobs` at a time, from the
    period `starts` names for it (its synthesis estimate) or from `start`; then every candidate at the kernel's iso
    period; then the controls named, at the iso period on stock OpenLane."""
    starts = starts or {}
    together(lambda c: search(kernel_, c, starts.get(c, start), deadline), gated_candidates(kernel_), jobs)
    iso(kernel_, deadline, jobs)
    period = iso_period(kernel_)
    together(lambda c: run(kernel_, c, period, deadline, "control"), list(controls), jobs)


def powers(kernel, period, deadline, jobs, only=()):
    """The kernel's power blocks, `jobs` at a time: every candidate that passed its gate (K7: each controller its
    traffic gate drives), or only those named in `only`. A name outside that set is refused before any block runs."""
    known_ = traffic_driven() if kernel == "K7" else gated_candidates(kernel)
    stray = [c for c in only if c not in known_]
    if stray:
        sys.exit("%s: %s %s no power block (the kernel's: %s)" % (kernel, ", ".join(stray), "has" if len(stray) == 1
                                                                   else "have", ", ".join(known_)))
    together(lambda c: power(kernel, c, period, deadline), [c for c in known_ if not only or c in only], jobs)


def kernels(names, jobs, start, deadline, controls=(), starts=None):
    """Several kernels whole at once, each as `kernel` runs it — a long kernel's flows overlap the short ones' rather
    than queueing behind them — every flow through this process's one start lock and BENCH_FLOWS. A kernel takes the
    controls named for it: a candidate belongs to the kernel its name begins with (k1_logos to K1)."""
    together(lambda k: kernel(k, jobs, start, deadline, [c for c in controls if c.split("_")[0] == k.lower()], starts),
             list(names), len(names))


def verify():
    bad = []
    for s in qorlib.load("sky130_stage", {"rows": []})["rows"]:
        d = os.path.join(STAGE_ROOT, s["block"])
        for f in s["files"]:
            p = os.path.join(d, f["name"])
            if not os.path.exists(p):
                bad.append("%s: %s is not staged (regenerate with: %s)" % (s["block"], f["name"], s["reproduce"]))
            elif qorlib.sha3_file(p) != f["sha3_256"]:
                bad.append("%s: %s differs from what the attempt staged" % (s["block"], f["name"]))
        if s["source_kind"] == "third-party" and s["kernel"] != "K7":
            manifest = {os.path.basename(f["path"]): f["sha3_256"] for c in qorlib.manifest()["candidates"]
                        if c["candidate"]["name"] == s["candidate"] for f in c["files"]}
            for f in s["files"][:-1]:
                if f.get("stand_in"):
                    if os.path.exists(os.path.join(d, f["name"])) and os.path.getsize(os.path.join(d, f["name"])):
                        bad.append("%s: %s stands in for an unread default and is not empty" % (s["block"], f["name"]))
                    continue
                if manifest.get(f["from"]) != f["sha3_256"]:
                    bad.append("%s: %s is not the manifest's %s" % (s["block"], f["name"], f["from"]))
    for b in bad:
        print("FAIL: " + b, file=sys.stderr)
    if bad:
        sys.exit(1)
    print("verified %d attempts" % len(qorlib.load("sky130_stage", {"rows": []})["rows"]))


def known(kernel, candidate):
    """Refuse a candidate the bench does not know by that name: one the functional gate passed in both modes, or a K7
    controller."""
    if kernel == "K7":
        ok = candidate in qorlib.fold_designs()
    else:
        modes = {r["mode"] for r in qorlib.load("function", {"rows": []})["rows"]
                 if (r["kernel"], r["candidate"]) == (kernel, candidate) and r["pass"]}
        ok = modes == {"shaped", "edge"}
    if not ok:
        sys.exit("%s %s: not a candidate that passed its functional gate (name it as the gate does, e.g. k1_logos)"
                 % (kernel, candidate))


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("mode", choices=["stage", "run", "search", "iso", "kernel", "kernels", "powers", "control", "power",
                                     "plan", "verify", "room"])
    ap.add_argument("kernel", nargs="?")
    ap.add_argument("candidate", nargs="?")
    ap.add_argument("--period", type=float)
    ap.add_argument("--at-iso", action="store_true", help="control: at the kernel's iso period")
    ap.add_argument("--start", type=float, default=5.0, help="search: the first period tried, in ns")
    ap.add_argument("--deadline", type=int, default=6 * 3600, help="seconds one OpenLane flow may take")
    ap.add_argument("--within", type=int, help="room: give up after this many seconds")
    ap.add_argument("--jobs", type=int, default=1, help="iso, kernel, powers: flows at once (BENCH_FLOWS lets them run)")
    ap.add_argument("--controls", default="", help="kernel: the candidates whose control runs at the iso period, "
                                                   "comma-separated")
    ap.add_argument("--candidates", default="", help="powers: only these candidates' power blocks, comma-separated")
    ap.add_argument("--start-from", help="kernel, kernels: a JSON file mapping a candidate to the period its search "
                                         "starts at (its synthesis estimate); --start for the rest")
    a = ap.parse_args()
    if a.mode == "room":
        global FLOW_WAIT_S
        if a.within is not None:
            FLOW_WAIT_S = a.within
        return room_for_a_flow()
    if a.mode == "plan":
        return plan()
    if a.mode == "verify":
        return verify()
    if a.mode == "iso":
        return iso(a.kernel or ap.error("name the kernel"), a.deadline, a.jobs)
    starts = json.load(open(a.start_from)) if a.start_from else None
    if a.mode == "kernel":
        return kernel(a.kernel or ap.error("name the kernel"), a.jobs, a.start, a.deadline,
                      [c for c in a.controls.split(",") if c], starts)
    if a.mode == "kernels":
        return kernels([k for k in (a.kernel or ap.error("name the kernels, comma-separated")).split(",") if k],
                       a.jobs, a.start, a.deadline, [c for c in a.controls.split(",") if c], starts)
    if a.mode == "powers":
        return powers(a.kernel or ap.error("name the kernel"), a.period, a.deadline, a.jobs,
                      [c for c in a.candidates.split(",") if c])
    if not (a.kernel and a.candidate):
        ap.error("name a kernel and a candidate")
    known(a.kernel, a.candidate)
    if a.mode == "search":
        return search(a.kernel, a.candidate, a.start, a.deadline)
    if a.mode == "power":
        return power(a.kernel, a.candidate, a.period, a.deadline)
    if a.at_iso:
        if a.mode != "control" or a.period is not None:
            ap.error("--at-iso places a control, instead of --period")
        a.period = iso_period(a.kernel)
    if a.period is None:
        ap.error("--period")
    if a.mode == "stage":
        print(stage(a.kernel, a.candidate, a.period)[0])
    else:
        run(a.kernel, a.candidate, a.period, a.deadline, "control" if a.mode == "control" else "attempt")


if __name__ == "__main__":
    main()
