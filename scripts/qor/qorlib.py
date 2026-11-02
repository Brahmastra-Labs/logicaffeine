"""Shared paths and ledger plumbing of the comparative-synthesis bench (scripts/qor/).

Every ledger is a JSON document `{_generated_by, schema, provenance, honesty, rows}` written only by its
recorder; each row carries the excerpts it was parsed from (committed under benchmarks/silicon/qor_logs/)
with their SHA3-256, so `verify-qor-ledgers.py` can re-parse every number on a fresh clone.
"""
import contextlib
import fcntl
import hashlib
import json
import os
import shutil
import subprocess
import sys
import tempfile
from datetime import datetime, timezone

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
SIL = os.path.join(ROOT, "benchmarks", "silicon")
EXCERPTS = os.path.join(SIL, "qor_logs")
LOGS = os.path.join(ROOT, "logs")
TARGET = os.environ.get("CARGO_TARGET_DIR", os.path.join(ROOT, "target"))
EMIT = os.path.join(TARGET, "debug", "examples", "emit_qor_harness")


# The floorplans a Sky130 run of the bench is tried in, in order: the first is openlane-run.sh's own, 35% core
# utilization at the compiler's measured 0.45 placement density. A run the global router refuses for congestion is
# repeated in the next, utilization and density lowered together — the placer spreads cells only as far as its target
# density, so a larger core alone leaves them as packed. One ladder serves every candidate.
SKY130_FLOORPLANS = (
    {"fp_core_util": 35, "pl_target_density": None},
    {"fp_core_util": 25, "pl_target_density": 0.35},
    {"fp_core_util": 15, "pl_target_density": 0.25},
)


def ledger_path(kind):
    return os.path.join(SIL, "qor_%s.json" % kind)


def run_root(ledger):
    """The OpenLane designs/ directory the runs of the Sky130 block ledger `ledger` are in: its run_root in
    benchmarks/silicon/sky130_ledgers.json, tools/OpenLane/designs when the entry names none."""
    name = os.path.relpath(ledger, ROOT)
    for led in json.load(open(os.path.join(SIL, "sky130_ledgers.json")))["ledgers"]:
        if led["path"] == name:
            return os.path.join(ROOT, led.get("run_root", os.path.join("tools", "OpenLane", "designs")))
    sys.exit("%s is not a registered block ledger (benchmarks/silicon/sky130_ledgers.json)" % name)


def sha3_text(text):
    return hashlib.sha3_256(text.encode()).hexdigest()


def sha3_file(path):
    return hashlib.sha3_256(open(path, "rb").read()).hexdigest()


def rtl_files(run_dir, rec):
    """The candidate RTL a run measured, as staged in `run_dir` (`rec["files"]`): each file's name and SHA3-256, the
    record that binds a row to the RTL behind it, as a Sky130 stage row's `files` does."""
    return [{"name": f, "sha3_256": sha3_file(os.path.join(run_dir, f))} for f in rec["files"]]


def manifest():
    """The candidate manifest scripts/qor/build-qor-candidate.py writes: every third-party candidate and refusal."""
    return json.load(open(os.path.join(SIL, "qor", "candidates.json")))


@contextlib.contextmanager
def scratch():
    """A working directory under logs/ (never /tmp, so a run's leftovers are where its logs are), removed after."""
    os.makedirs(LOGS, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=LOGS, prefix="qor-") as d:
        yield d


def rel(path):
    return os.path.relpath(path, ROOT)


def now():
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def load(kind, header):
    path = ledger_path(kind)
    if os.path.exists(path):
        return json.load(open(path))
    doc = dict(header)
    doc["rows"] = []
    return doc


def save(kind, doc):
    """Write the ledger whole or not at all: a reader never sees half a file."""
    doc["rows"] = sorted(doc["rows"], key=lambda r: r["key"])
    tmp = os.path.join(LOGS, "qor_%s.json.tmp" % kind)
    with open(tmp, "w") as f:
        json.dump(doc, f, indent=1, sort_keys=True)
        f.write("\n")
    os.replace(tmp, ledger_path(kind))


def put_row(doc, row):
    doc["rows"] = [r for r in doc["rows"] if r["key"] != row["key"]] + [row]


@contextlib.contextmanager
def locked(kind):
    """Hold the ledger's lock: every recorder that updates `kind` serialises here, so two recorders running
    at once can never write over each other's rows."""
    with open(os.path.join(LOGS, "qor_%s.lock" % kind), "w") as f:
        fcntl.flock(f, fcntl.LOCK_EX)
        try:
            yield
        finally:
            fcntl.flock(f, fcntl.LOCK_UN)


def update(kind, header, row, excerpts=None):
    """Put `row` into ledger `kind` under its lock — load, replace the row with its key, save — and commit the
    `excerpts` (key, [(name, text)]) it names in the same hold: a reader holding the lock sees every row with the
    excerpts it names, never a run's excerpts beside the row of the run before. A row whose excerpts are not exactly
    the ones given is refused before anything is written."""
    named = (row.get("excerpts", {}), row.get("excerpt_sha3", {}))
    given = excerpt_record(*excerpts) if excerpts is not None else ({}, {})
    if named != given:
        sys.exit("%s: the row names excerpts %s, the run gives %s" % (row.get("key"), sorted(named[0]), sorted(given[0])))
    with locked(kind):
        if excerpts is not None:
            write_excerpts(*excerpts)
        doc = load(kind, header)
        put_row(doc, row)
        save(kind, doc)


def excerpt_record(key, files):
    """The ({name: path}, {name: sha3}) each (name, text) of `files` will have under qor_logs/<key>/ — what a row names
    before `update` commits the files with it."""
    return ({name: rel(os.path.join(EXCERPTS, key, name)) for name, _ in files},
            {name: sha3_text(text) for name, text in files})


def write_excerpts(key, files):
    """Commit each (name, text) under qor_logs/<key>/ and return ({name: path}, {name: sha3}). The directory then holds
    exactly those files — a run recorded again leaves nothing of the run before — and a run with none has none.
    Recorders reach it through `update`, under the ledger's lock."""
    dest = os.path.join(EXCERPTS, key)
    if os.path.isdir(dest):
        shutil.rmtree(dest)
    paths, digests = {}, {}
    if not files:
        return paths, digests
    os.makedirs(dest)
    for name, text in files:
        p = os.path.join(dest, name)
        with open(p, "w") as f:
            f.write(text)
        paths[name] = rel(p)
        digests[name] = sha3_text(text)
    return paths, digests


def read_excerpts(row):
    """Every excerpt of `row` as text, or the list of problems (missing, edited)."""
    texts, bad = {}, []
    for name, path in row["excerpts"].items():
        p = os.path.join(ROOT, path)
        if not os.path.exists(p):
            bad.append("%s: excerpt %s missing" % (row["key"], name))
            continue
        text = open(p).read()
        if sha3_text(text) != row["excerpt_sha3"][name]:
            bad.append("%s: excerpt %s was edited" % (row["key"], name))
            continue
        texts[name] = text
    return texts, bad


def orphan_excerpts(prefix, rows):
    """Excerpt directories under qor_logs/<prefix>/ that no ledger row names: evidence of a run whose row is gone."""
    root = os.path.join(EXCERPTS, prefix)
    named = {os.path.dirname(p) for r in rows for p in r["excerpts"].values()}
    if not os.path.isdir(root):
        return []
    return sorted(rel(os.path.join(root, d)) for d in os.listdir(root) if rel(os.path.join(root, d)) not in named)


def emit(run_dir, kernel, candidate, mode, ops, module=None):
    """Materialise `candidate` (or `logos`) of `kernel` into `run_dir` with emit_qor_harness; with `module`, its harness
    under that name."""
    if not os.path.exists(EMIT):
        sys.exit("build the emitter first: cargo build -p logicaffeine-compile --example emit_qor_harness")
    r = subprocess.run([EMIT, run_dir, kernel, candidate, mode, str(ops)] + (["--as", module] if module else []),
                       capture_output=True, text=True)
    if r.returncode != 0:
        sys.exit("emit_qor_harness: %s" % r.stderr.strip())
    return json.load(open(os.path.join(run_dir, "candidate.json")))


EMIT_FOLD = os.path.join(TARGET, "debug", "examples", "emit_qor_fold")
LITEX_PYTHON = os.path.join(ROOT, "tools", "qor", "litex-venv", "bin", "python")


def fold_designs():
    """The designs measured without a functional contract (kernels/K7/kernel.json), by name."""
    return json.load(open(os.path.join(SIL, "qor", "kernels", "K7", "kernel.json")))["designs"]


def fold_generate(name):
    """The command that generates fold design `name`'s RTL, as a reader would run it (`<out.v>` the file it writes)."""
    d = fold_designs()[name]
    if d["source"] == "logos":
        return "cargo run -p logicaffeine-compile --example emit_qor_fold -- rtl %s <out.v>" % d["top"]
    return "tools/qor/litex-venv/bin/python %s <out.v>" % rel(os.path.join(SIL, "qor", "kernels", "K7", d["script"]))


def emit_fold(run_dir, name):
    """Materialise fold design `name` into `run_dir`: its RTL, its `qorsyn_<name>` wrapper. Returns the record the
    harnessed candidates' candidate.json carries, plus the RTL's SHA3-256 and the command that generated it."""
    d = fold_designs()[name]
    if not os.path.exists(EMIT_FOLD):
        sys.exit("build the emitter first: cargo build -p logicaffeine-compile --example emit_qor_fold")
    rtl = os.path.join(run_dir, "%s.v" % d["top"])
    if d["source"] == "logos":
        cmd = [EMIT_FOLD, "rtl", d["top"], rtl]
    else:
        cmd = [LITEX_PYTHON, os.path.join(SIL, "qor", "kernels", "K7", d["script"]), rtl]
    generate = fold_generate(name)
    r = subprocess.run(cmd, capture_output=True, text=True)
    if r.returncode != 0:
        sys.exit("%s: %s" % (name, (r.stdout + r.stderr).strip()[-2000:]))
    ports = os.path.join(run_dir, "ports.json")
    r = subprocess.run(["yosys", "-q", "-p", "read_verilog %s; hierarchy -top %s; proc; write_json %s" % (rtl, d["top"], ports)],
                       capture_output=True, text=True)
    if r.returncode != 0:
        sys.exit("%s: yosys refused the RTL:\n%s" % (name, r.stderr[-2000:]))
    wrapper = os.path.join(run_dir, "qorsyn_%s.v" % name)
    r = subprocess.run([EMIT_FOLD, "wrap", name, d["top"], d["clock"], d["reset"], d["polarity"], ports, wrapper],
                       capture_output=True, text=True)
    if r.returncode != 0:
        sys.exit("%s: %s" % (name, r.stderr.strip()))
    text = open(rtl).read()
    return {"kernel": "K7", "kernel_name": "ddr2_controller", "candidate": {"name": name, "top": d["top"]},
            "source_kind": "LOGOS RTL generator" if d["source"] == "logos" else "third-party",
            "files": [os.path.basename(rtl)], "harness": os.path.basename(wrapper), "harness_top": "qorsyn_%s" % name,
            "rtl_sha3_256": sha3_text(text), "generate": generate, "note": d["note"]}


def wrap_candidate(run_dir):
    """The fold wrapper of the candidate emit() materialised into `run_dir`: the harness entries of its record,
    pointing at `qorsyn_<candidate>` instead of the functional harness."""
    if not os.path.exists(EMIT_FOLD):
        sys.exit("build the emitter first: cargo build -p logicaffeine-compile --example emit_qor_fold")
    record = os.path.join(run_dir, "candidate.json")
    name = json.load(open(record))["candidate"]["name"]
    wrapper = os.path.join(run_dir, "qorsyn_%s.v" % name)
    r = subprocess.run([EMIT_FOLD, "wrap-candidate", record, wrapper], capture_output=True, text=True)
    if r.returncode != 0:
        sys.exit("emit_qor_fold: %s" % r.stderr.strip())
    return {"harness": os.path.basename(wrapper), "harness_top": "qorsyn_%s" % name}


# The compiler's flow knowledge: the sources the flow config of every run is emitted from (scripts/openlane-run.sh
# refuses an emitter older than any of them).
# The compiler sources a Sky130 run's flow config is emitted from: the emitter is refused when any is newer than it.
FLOW_KNOWLEDGE = ("crates/logicaffeine_compile/src/codegen_sva/flow_config.rs",
                  "crates/logicaffeine_compile/src/codegen_sva/pdk_target.rs",
                  "crates/logicaffeine_compile/src/codegen_sva/signoff_blocks.rs",
                  "crates/logicaffeine_compile/examples/emit_flow_config.rs")
EMIT_FLOW_CONFIG = os.path.join(TARGET, "debug", "examples", "emit_flow_config")
# The fields of an emitted flow config that are the design's own, not the flow's.
DESIGN_FIELDS = ("DESIGN_NAME", "CLOCK_PORT", "CLOCK_PERIOD")


def routing_cores():
    """The detailed router's threads a bench flow runs with (detailed routing is deterministic per thread count)."""
    return os.environ.get("ROUTING_CORES", "8")


def flow_settings_config():
    """The flow settings the compiler gives a bench block: the config emit_flow_config emits for a comparative probe
    block at the bench's routing cores, its design's own fields left out."""
    if not os.path.exists(EMIT_FLOW_CONFIG):
        sys.exit("build the flow-config emitter first: cargo build -p logicaffeine-compile --example emit_flow_config")
    newer = [p for p in FLOW_KNOWLEDGE if os.path.getmtime(os.path.join(ROOT, p)) > os.path.getmtime(EMIT_FLOW_CONFIG)]
    if newer:
        sys.exit("%s is newer than the flow-config emitter: build it first (cargo build -p logicaffeine-compile "
                 "--example emit_flow_config)" % ", ".join(newer))
    r = subprocess.run([EMIT_FLOW_CONFIG, "qor_flow_probe", "clk", "10", "--pdk", "sky130A", "--cores", routing_cores(),
                        "--comparative"], capture_output=True, text=True)
    if r.returncode != 0:
        sys.exit("emit_flow_config: %s" % r.stderr.strip())
    return {k: v for k, v in json.loads(r.stdout).items() if k not in DESIGN_FIELDS}


def flow_settings():
    """The SHA3-256 of the flow settings the compiler gives a bench block (flow_settings_config), as
    json.dumps(sort_keys=True) prints them. It moves with every setting a bench block gets, and with nothing else — a
    signoff block re-recorded, a comment rewritten."""
    return sha3_text(json.dumps(flow_settings_config(), sort_keys=True))


def openroad_image():
    """The OpenLane image every bench run and control flows in: the image at the overlay's base with its OpenROAD
    rebuilt with the bench's one fix, named as scripts/openroad/build-openroad.sh names it — by the base and the SHA-256
    of its Dockerfile, patch and source list together."""
    here = os.path.join(ROOT, "scripts", "openroad")
    built = hashlib.sha256(b"".join(open(os.path.join(here, f), "rb").read()
                                    for f in ("openroad.Dockerfile", "looped-route.patch", "sources.txt")))
    base = open(os.path.join(ROOT, "scripts", "openlane-overlay", "BASE")).read().strip()
    return "logos-openlane:%s-openroad-%s" % (base[:8], built.hexdigest()[:12])


IR_ANALYSIS = ("scripts/anchor-ir-drop.sh", "scripts/gen-vsrc-loc.py")


def ir_analysis():
    """The SHA3-256 over the IR analysis a run's verdict gates on (path and content of each file, in path order):
    scripts/anchor-ir-drop.sh, which replays the run's irdrop step with anchored sources, and scripts/gen-vsrc-loc.py,
    which places them. Its output is a report frozen in the run directory, which no re-derivation of the record can
    see change — unlike the recorder's verdicts, which every verify re-derives."""
    return hashlib.sha3_256("".join("%s\0%s\n" % (f, sha3_file(os.path.join(ROOT, f)))
                                    for f in sorted(IR_ANALYSIS)).encode()).hexdigest()


def flow_identity(stock):
    """The flow a Sky130 run goes through: the OpenLane commit; the image it runs (openroad_image); the SHA3-256 over
    the repository's overlay (path and content of each file, in path order), none for a run on stock OpenLane; the
    flow settings the compiler gives a bench block (flow_settings); and the IR analysis its verdict gates on
    (ir_analysis). Two runs compare only through one identity."""
    overlay = os.path.join(ROOT, "scripts", "openlane-overlay")
    base = open(os.path.join(overlay, "BASE")).read().strip()
    settings = flow_settings()
    image = openroad_image()
    analysis = ir_analysis()
    if stock:
        return {"openlane": base, "openroad_image": image, "overlay_sha3": None, "flow_settings_sha3": settings,
                "ir_analysis_sha3": analysis}
    files = [os.path.join(d, n) for d, _, names in os.walk(os.path.join(overlay, "scripts")) if "__pycache__" not in d
             for n in names]
    return {"openlane": base, "openroad_image": image, "overlay_sha3": hashlib.sha3_256("".join(sorted(
        "%s\0%s\n" % (os.path.relpath(f, overlay), sha3_file(f)) for f in files)).encode()).hexdigest(),
        "flow_settings_sha3": settings, "ir_analysis_sha3": analysis}


def logos_candidates():
    """Every LOGOS candidate as (kernel, name), from emit_qor_harness --list."""
    if not os.path.exists(EMIT):
        sys.exit("build the emitter first: cargo build -p logicaffeine-compile --example emit_qor_harness")
    out = subprocess.run([EMIT, "--list"], capture_output=True, text=True, check=True).stdout
    return [tuple(l.split()) for l in out.splitlines() if l.strip()]


def tool_version(cmd):
    r = subprocess.run(cmd, capture_output=True, text=True)
    return (r.stdout + r.stderr).strip().splitlines()[0] if (r.stdout + r.stderr).strip() else "unknown"
